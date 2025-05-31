use crate::auth::jwt;
use crate::proxy::services::common::{error_response, success_response};
use axum::extract::Extension;
use axum::http::{StatusCode, HeaderMap};
use axum::response::IntoResponse;
use axum::Json;
use common::config::ConfigLoader;
use common::error::Error;
use common::grpc_client::UserServiceGrpcClient;
use common::proto::user::user_service_client::UserServiceClient;
use common::proto::user::VerifyPasswordRequest;
use common::service_discovery::LbWithServiceDiscovery;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use common::auth::jwt::UserInfo;

/// 平台类型枚举
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlatformType {
    Web,
    Mobile,
    IOS,
    Android,
    Desktop,
    Unknown,
}

impl From<&str> for PlatformType {
    fn from(system_type: &str) -> Self {
        match system_type.to_lowercase().as_str() {
            "web" | "browser" => PlatformType::Web,
            "mobile" | "app" => PlatformType::Mobile,
            "ios" => PlatformType::IOS,
            "android" => PlatformType::Android,
            "desktop" | "pc" | "windows" | "macos" | "linux" => PlatformType::Desktop,
            _ => PlatformType::Unknown,
        }
    }
}

impl PlatformType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlatformType::Web => "web",
            PlatformType::Mobile => "mobile",
            PlatformType::IOS => "ios",
            PlatformType::Android => "android",
            PlatformType::Desktop => "desktop",
            PlatformType::Unknown => "unknown",
        }
    }
}

/// 登录请求
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// 用户名
    pub username: String,
    /// 密码
    pub password: String,
    /// 租户ID
    pub tenant_id: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginByPhoneRequest {
    /// 手机号
    pub phone: String,
    /// 验证码
    pub verify_code: String,
    /// 租户ID
    pub tenant_id: String,
}

/// 登录响应
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// 访问令牌
    pub access_token: String,
    /// 刷新令牌
    pub refresh_token: String,
    /// 令牌类型
    pub token_type: String,
    /// 过期时间（秒）
    pub expires_in: u64,
    /// 用户信息
    pub user_info: UserInfoResponse,
}

/// 刷新令牌请求
#[derive(Debug, Deserialize)]
pub struct RefreshTokenRequest {
    /// 刷新令牌
    pub refresh_token: String,
}

/// 用户信息响应
#[derive(Debug, Serialize)]
pub struct UserInfoResponse {
    /// 用户ID
    pub user_id: String,
    /// 用户名
    pub username: String,
    /// 租户ID
    pub tenant_id: String,
    /// 租户名称
    pub tenant_name: String,
    /// 用户邮箱
    pub email: Option<String>,
    /// 用户昵称
    pub nickname: Option<String>,
    /// 头像URL
    pub avatar_url: Option<String>,
    /// 密信id
    pub custom_id: Option<String>,
}

/// 从请求头中提取平台信息
fn extract_platform_info(headers: &HeaderMap) -> PlatformType {
    let system_type = headers
        .get("system-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown");

    let platform = PlatformType::from(system_type);
    
    debug!("检测到登录平台: {:?}, 原始system-type: {}", platform, system_type);

    platform
}

/// 用户服务共享实例
#[derive(Clone)]
pub struct SharedUserService(UserServiceClient<LbWithServiceDiscovery>);

impl SharedUserService {
    /// 创建新的共享用户服务
    pub fn new(client: UserServiceClient<LbWithServiceDiscovery>) -> Self {
        Self(client)
    }

    /// 验证密码
    pub async fn verify_password(
        &self,
        request: VerifyPasswordRequest,
    ) -> Result<common::proto::user::VerifyPasswordResponse, anyhow::Error> {
        // 克隆基础客户端
        let mut client = self.0.clone();
        client
            .verify_password(request)
            .await
            .map(|response| response.into_inner())
            .map_err(Into::into)
    }

    /// 验证手机验证码登录
    pub async fn verify_phone_code_login(
        &self,
        request: common::proto::user::VerifyPhoneCodeRequest,
    ) -> Result<common::proto::user::VerifyPasswordResponse, anyhow::Error> {
        // 克隆基础客户端
        let mut client = self.0.clone();
        client
            .verify_phone_code_login(request)
            .await
            .map(|response| response.into_inner())
            .map_err(Into::into)
    }
}

/// 处理短信验证码登录请求
pub async fn login_by_phone(
    Extension(user_service): Extension<SharedUserService>,
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    Json(login_req): Json<LoginByPhoneRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("短信验证码登录请求：手机号 {}", login_req.phone);

    // 提取平台信息
    let platform = extract_platform_info(&headers);

    // 创建验证请求
    let verify_request = common::proto::user::VerifyPhoneCodeRequest {
        phone: login_req.phone.clone(),
        code: login_req.verify_code,
        action: "login".to_string(),
    };

    // 调用用户服务验证手机验证码
    let response = match user_service.verify_phone_code_login(verify_request).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("调用用户服务验证手机验证码失败: {}", e);
            return Ok(error_response(
                &format!("验证手机验证码服务错误:{}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    // 检查验证是否有效
    if !response.valid || response.user.is_none() {
        return Err(Error::Authentication("手机号或验证码不正确".to_string()));
    }

    // 获取用户信息
    let user = response.user.unwrap();

    info!("用户 {} 手机号登录成功，平台: {}", user.username, platform.as_str());

    // 提取用户额外信息
    let extra = extract_user_extra(&user);

    // 将user.id (String类型) 转换为i64
    let user_id = user
        .id
        .parse::<i64>()
        .map_err(|_| Error::Internal("无法解析用户ID".to_string()))?;

    // 构建登录响应
    let login_response = build_login_response(
        user_id,
        &user.username,
        // 简化示例，在实际应用中应从用户信息中获取租户ID和名称
        1,         // 示例租户ID
        "default", // 示例租户名称，实际应从用户信息中获取
        extra,
        cache_instance,
        platform,
    )
    .await?;

    // 返回响应
    Ok(success_response(login_response, StatusCode::OK))
}

/// 处理登录请求
pub async fn login(
    Extension(user_service): Extension<SharedUserService>,
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    Json(login_req): Json<LoginRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("登录请求：用户 {}", login_req.username);

    // 提取平台信息
    let platform = extract_platform_info(&headers);

    // 创建验证密码请求
    let verify_request = VerifyPasswordRequest {
        username: login_req.username.clone(),
        password: login_req.password,
    };

    // 调用用户服务验证密码
    let response = match user_service.verify_password(verify_request).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("调用用户服务验证密码失败: {}", e);
            return Ok(error_response(
                &format!("验证密码服务错误:{}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    // 检查密码是否有效
    if !response.valid || response.user.is_none() {
        return Err(Error::Internal("用户名或密码不正确".to_string()));
    }

    // 获取用户信息
    let user = response.user.unwrap();

    info!("用户 {} 登录成功，平台: {}", login_req.username, platform.as_str());

    // 提取用户额外信息
    let extra = extract_user_extra(&user);

    // 将user.id (String类型) 转换为i64
    let user_id = user
        .id
        .parse::<i64>()
        .map_err(|_| Error::Internal("无法解析用户ID".to_string()))?;

    // 构建登录响应
    let login_response = build_login_response(
        user_id,
        &user.username,
        // 简化示例，在实际应用中应从用户信息中获取租户ID和名称
        1,         // 示例租户ID
        "default", // 示例租户名称，实际应从用户信息中获取
        extra,
        cache_instance,
        platform,
    )
    .await?;

    // 返回响应
    Ok(success_response(login_response, StatusCode::OK))
}

/// 处理令牌刷新请求
pub async fn refresh_token(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    Json(refresh_req): Json<RefreshTokenRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("刷新令牌请求");

    // 提取平台信息
    let platform = extract_platform_info(&headers);

    // 读取JWT配置
    let config = ConfigLoader::get_global().expect("Failed to get global config");

    let jwt_config = &config.gateway.auth.jwt;

    // 验证刷新令牌
    let user_info = jwt::verify_token(&refresh_req.refresh_token, jwt_config)?;

    // 构建额外信息
    let extra = user_info.extra.clone();

    // 获取用户信息用于日志
    let username = user_info.username.clone();

    // 构建登录响应
    let refresh_response = build_login_response(
        user_info.user_id,
        &user_info.username,
        user_info.tenant_id,
        &user_info.tenant_name,
        extra,
        cache_instance,
        platform,
    )
    .await?;

    info!("用户 {} 刷新令牌成功，平台: {}", username, platform);

    // 返回响应
    Ok(success_response(refresh_response, StatusCode::OK))
}

/// 从用户信息中提取额外数据
fn extract_user_extra(user: &common::proto::user::User) -> HashMap<String, String> {
    let mut extra = HashMap::new();

    // email在proto中是String类型，但我们需要考虑其可能为空的情况
    if !user.email.is_empty() {
        extra.insert("email".to_string(), user.email.clone());
    }

    // 如果存在昵称，添加到额外信息中
    if let Some(nickname) = &user.nickname {
        extra.insert("nickname".to_string(), nickname.clone());
    }

    // 如果存在头像URL，添加到额外信息中
    if let Some(avatar_url) = &user.avatar_url {
        extra.insert("avatar_url".to_string(), avatar_url.clone());
    }

    // 如果存在密信id，添加到额外信息中
    if !user.custom_id.is_empty() {
        extra.insert("custom_id".to_string(), user.custom_id.clone());
    }

    extra
}

/// 构建登录响应
async fn build_login_response(
    user_id: i64,
    username: &str,
    tenant_id: i64,
    tenant_name: &str,
    extra: HashMap<String, String>,
    cache_instance: Arc<dyn cache::Cache>,
    platform: PlatformType
) -> Result<LoginResponse, Error> {
    // 读取JWT配置
    let config = ConfigLoader::get_global().expect("Failed to get global config");

    let jwt_config = &config.gateway.auth.jwt;

    // 在额外信息中添加平台信息
    let mut extended_extra = extra.clone();
    extended_extra.insert("platform".to_string(), platform.as_str().to_string());

    // 生成访问令牌
    let access_token = jwt::generate_token(
        user_id,
        username,
        tenant_id,
        tenant_name,
        extended_extra.clone(),
        jwt_config,
    )?;

    // 生成刷新令牌
    let refresh_token =
        jwt::generate_refresh_token(user_id, username, tenant_id, tenant_name, extended_extra.clone(),jwt_config)?;

    // 将用户标记为在线状态
    if let Err(e) = cache_instance.user_login(&user_id.to_string()).await {
        error!("将用户{}标记为在线状态失败: {}", user_id, e);
        // 不影响登录流程，继续执行
    } else {
        debug!("用户{}已标记为在线状态", user_id);
    }

    // 为不同平台创建不同的token键，支持多平台同时登录
    let platform_str = platform.as_str();
    
    // 将访问令牌存储到Redis中，按平台分别存储
    if let Err(e) = cache_instance.save_access_token_for_platform(
        &user_id.to_string(), 
        &access_token, 
        platform_str,
        jwt_config.expiry_seconds
    ).await {
        error!("存储用户{}在{}平台的访问令牌失败: {}", user_id, platform_str, e);
        // 不影响登录流程，继续执行
    } else {
        debug!("用户{}在{}平台的访问令牌已存储到Redis", user_id, platform_str);
    }

    // 将刷新令牌存储到Redis中，按平台分别存储
    if let Err(e) = cache_instance.save_refresh_token_for_platform(
        &user_id.to_string(), 
        &refresh_token,
        platform_str,
        jwt_config.refresh_expiry_seconds
    ).await {
        error!("存储用户{}在{}平台的刷新令牌失败: {}", user_id, platform_str, e);
        // 不影响登录流程，继续执行
    } else {
        debug!("用户{}在{}平台的刷新令牌已存储到Redis", user_id, platform_str);
    }

    // 构建用户信息响应
    let user_info = UserInfoResponse {
        user_id: user_id.to_string(),
        username: username.to_string(),
        tenant_id: tenant_id.to_string(),
        tenant_name: tenant_name.to_string(),
        email: extra.get("email").cloned(),
        custom_id: extra.get("custom_id").cloned(),
        nickname: extra.get("nickname").cloned(),
        avatar_url: extra.get("avatar_url").cloned(),
    };

    // 构建登录响应
    let login_response = LoginResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: jwt_config.expiry_seconds,
        user_info,
    };

    Ok(login_response)
}

/// 处理用户登出请求（支持指定平台登出）
pub async fn logout(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, Error> {
    debug!("用户登出请求：用户ID {}", user_id);
    // 获取token
    // 提取平台信息
    let platform = extract_platform_info(&headers);
    let platform_str = platform.as_str();

    // 删除指定平台的访问令牌
    if let Err(e) = cache_instance.delete_access_token_for_platform(&user_id, platform_str).await {
        error!("删除用户{}在{}平台的访问令牌失败: {}", user_id, platform_str, e);
    } else {
        debug!("用户{}在{}平台的访问令牌已删除", user_id, platform_str);
    }

    // 删除指定平台的刷新令牌
    if let Err(e) = cache_instance.delete_refresh_token_for_platform(&user_id, platform_str).await {
        error!("删除用户{}在{}平台的刷新令牌失败: {}", user_id, platform_str, e);
    } else {
        debug!("用户{}在{}平台的刷新令牌已删除", user_id, platform_str);
    }

    // 检查用户是否还有其他平台的登录token，如果没有则清理在线状态
    let has_other_tokens = cache_instance.check_user_has_any_tokens(&user_id).await.unwrap_or(true);
    if !has_other_tokens {
        if let Err(e) = cache_instance.user_logout(&user_id).await {
            error!("清理用户{}在线状态失败: {}", user_id, e);
        } else {
            debug!("用户{}已从在线状态中移除", user_id);
        }
    }

    info!("用户{}从{}平台登出成功", user_id, platform_str);

    // 返回成功响应
    Ok(success_response(
        serde_json::json!({
            "message": "登出成功",
            "platform": platform_str
        }),
        StatusCode::OK
    ))
}

/// 处理用户全平台登出请求
pub async fn logout_all_platforms(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, Error> {
    debug!("用户全平台登出请求：用户ID {}", user_id);

    // 清理用户的在线状态
    if let Err(e) = cache_instance.user_logout(&user_id).await {
        error!("清理用户{}在线状态失败: {}", user_id, e);
    } else {
        debug!("用户{}已从在线状态中移除", user_id);
    }

    // 删除所有平台的令牌
    if let Err(e) = cache_instance.delete_all_user_tokens(&user_id).await {
        error!("删除用户{}所有平台令牌失败: {}", user_id, e);
    } else {
        debug!("用户{}所有平台的令牌已删除", user_id);
    }

    info!("用户{}全平台登出成功", user_id);

    // 返回成功响应
    Ok(success_response(
        serde_json::json!({
            "message": "全平台登出成功"
        }),
        StatusCode::OK
    ))
}

/// 获取用户在线平台信息
pub async fn get_user_platforms(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, Error> {
    debug!("获取用户{}的在线平台信息", user_id);

    // 获取用户在所有平台的登录信息
    let platforms = match cache_instance.get_user_login_platforms(&user_id).await {
        Ok(platforms) => platforms,
        Err(e) => {
            error!("获取用户{}的平台信息失败: {}", user_id, e);
            return Ok(error_response(
                &format!("获取平台信息失败: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    debug!("用户{}当前在线平台: {:?}", user_id, platforms);

    // 返回成功响应
    Ok(success_response(
        serde_json::json!({
            "user_id": user_id,
            "platforms": platforms,
            "platform_count": platforms.len()
        }),
        StatusCode::OK
    ))
}