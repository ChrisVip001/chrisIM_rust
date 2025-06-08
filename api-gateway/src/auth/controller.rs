use crate::proxy::services::common::{error_response, success_response};
use axum::body::Body;
use axum::extract::Extension;
use axum::http::{HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use common::auth::{jwt, Claims};
use common::config::ConfigLoader;
use common::error::Error;
use common::proto::message::PlatformType;
use common::proto::user::user_service_client::UserServiceClient;
use common::proto::user::VerifyPasswordRequest;
use common::service_discovery::LbWithServiceDiscovery;
use common::utils::verify_image_code;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

/// 错误wrapper，实现IntoResponse trait
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        error_response(&self.0.to_string(), StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
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
    /// 图片验证码
    pub image_code: String,
    /// 图片验证码Key
    pub image_code_key: String,
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

    let platform = PlatformType::from_str_name(system_type).expect("无效的平台！");

    debug!(
        "检测到登录平台: {:?}, 原始system-type: {}",
        platform, system_type
    );

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
#[axum::debug_handler]
pub async fn login_by_phone(
    Extension(user_service): Extension<SharedUserService>,
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    Json(login_req): Json<LoginByPhoneRequest>,
) -> Result<impl IntoResponse, AppError> {
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
    let response = user_service.verify_phone_code_login(verify_request).await?;

    // 检查验证是否有效
    if !response.valid || response.user.is_none() {
        return Ok(error_response(
            "手机号或验证码不正确",
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }

    // 获取用户信息
    let user = response.user.unwrap();
    if login_req.tenant_id != user.tenant_id {
        return Ok(error_response(
            "企业号错误",
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }
    info!(
        "用户 {} 手机号登录成功，平台: {}",
        user.username,
        PlatformType::as_str_name(&platform)
    );

    // 提取用户额外信息
    let extra = extract_user_extra(&user);

    // 构建登录响应
    let login_response = build_login_response(
        &user.id,
        &user.username,
        user.tenant_id.clone(),
        "default",
        extra,
        cache_instance,
        platform as i32,
    )
    .await?;

    // 返回响应
    Ok(success_response(login_response, StatusCode::OK))
}

/// 处理登录请求
#[axum::debug_handler]
pub async fn login(
    Extension(user_service): Extension<SharedUserService>,
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    Json(login_req): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    debug!("登录请求：用户 {}", login_req.username);

    // 提取平台信息
    let platform = extract_platform_info(&headers);

    // 创建验证密码请求
    let verify_request = VerifyPasswordRequest {
        username: login_req.username.clone(),
        password: login_req.password,
    };

    // 图片验证码校验
    if !verify_image_code(&login_req.image_code_key, &login_req.image_code) {
        return Ok(error_response(
            "图片验证码错误",
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }

    // 调用用户服务验证密码
    let response = user_service.verify_password(verify_request).await?;

    // 检查密码是否有效
    if !response.valid || response.user.is_none() {
        return Ok(error_response(
            "用户名或密码不正确",
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }

    // 获取用户信息
    let user = response.user.unwrap();

    if login_req.tenant_id != user.tenant_id {
        return Ok(error_response(
            "企业号错误",
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }
    info!(
        "用户 {} 登录成功，平台: {}",
        login_req.username,
        PlatformType::as_str_name(&platform)
    );

    // 提取用户额外信息
    let extra = extract_user_extra(&user);

    // 构建登录响应
    let login_response = build_login_response(
        &user.id,
        &user.username,
        user.tenant_id,
        "default",
        extra,
        cache_instance,
        platform as i32,
    )
    .await?;

    // 返回响应
    Ok(success_response(login_response, StatusCode::OK))
}

/// 处理令牌刷新请求
#[axum::debug_handler]
pub async fn refresh_token(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    headers: HeaderMap,
    Json(refresh_req): Json<RefreshTokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    debug!("刷新令牌请求");

    // 提取平台信息
    let platform = extract_platform_info(&headers);

    // 读取JWT配置
    let config = ConfigLoader::get_global().expect("Failed to get global config");

    let jwt_config = &config.gateway.auth.jwt;

    // 验证刷新令牌
    let user_info = jwt::verify_token(&refresh_req.refresh_token, jwt_config)?;

    // 验证Redis中的刷新令牌是否存在且匹配
    match cache_instance
        .get_refresh_token_for_platform(&user_info.sub, platform as i32)
        .await
    {
        Ok(Some(stored_token)) => {
            if stored_token != refresh_req.refresh_token {
                return Ok(error_response(
                    "令牌已过期或已注销，请重新登录",
                    StatusCode::UNAUTHORIZED,
                ));
            }
        }
        Ok(None) => {
            return Ok(error_response(
                "令牌已过期或已注销，请重新登录",
                StatusCode::UNAUTHORIZED,
            ));
        }
        Err(_) => {
            return Ok(error_response("令牌验证服务错误", StatusCode::UNAUTHORIZED));
        }
    }
    // 构建额外信息
    let extra = user_info.extra.clone();

    // 构建登录响应
    let refresh_response = build_login_response(
        &user_info.sub,
        &user_info.username,
        user_info.tenant_id,
        &user_info.tenant_name,
        extra,
        cache_instance,
        platform as i32,
    )
    .await?;

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
    user_id: &str,
    username: &str,
    tenant_id: String,
    tenant_name: &str,
    extra: HashMap<String, String>,
    cache_instance: Arc<dyn cache::Cache>,
    platform: i32,
) -> Result<LoginResponse, Error> {
    // 读取JWT配置
    let config = ConfigLoader::get_global().expect("Failed to get global config");

    let jwt_config = &config.gateway.auth.jwt;

    // 生成访问令牌
    let access_token = jwt::generate_token(
        user_id,
        username,
        tenant_id.clone(),
        tenant_name,
        platform,
        extra.clone(),
        jwt_config,
    )?;

    // 生成刷新令牌
    let refresh_token = jwt::generate_refresh_token(
        &user_id,
        username,
        tenant_id.clone(),
        tenant_name,
        platform,
        extra.clone(),
        jwt_config,
    )?;

    // 将用户在指定平台标记为在线状态
    cache_instance
        .user_platform_login(&user_id, platform)
        .await?;

    // 将访问令牌存储到Redis中，按平台分别存储
    cache_instance
        .save_access_token_for_platform(
            &user_id,
            &access_token,
            platform,
            jwt_config.expiry_seconds,
        )
        .await?;

    // 将刷新令牌存储到Redis中，按平台分别存储
    cache_instance
        .save_refresh_token_for_platform(
            &user_id,
            &refresh_token,
            platform,
            jwt_config.refresh_expiry_seconds,
        )
        .await?;

    // 构建登录响应
    let login_response = LoginResponse {
        access_token,
        refresh_token,
        token_type: jwt_config.header_prefix.clone(),
        expires_in: jwt_config.expiry_seconds,
    };

    Ok(login_response)
}

/// 处理登出请求
#[axum::debug_handler]
pub async fn logout(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    Extension(user_info): Extension<Claims>,
) -> Result<impl IntoResponse, AppError> {
    debug!("登出请求：用户 {}", user_info.username);

    // 退出指定平台登录状态
    cache_instance
        .user_platform_logout(&user_info.sub, user_info.platform)
        .await?;

    // 清除访问令牌
    cache_instance
        .delete_access_token_for_platform(&user_info.sub, user_info.platform)
        .await?;

    // 清除刷新令牌
    cache_instance
        .delete_refresh_token_for_platform(&user_info.sub, user_info.platform)
        .await?;

    info!(
        "用户 {} 登出成功，平台: {}",
        user_info.username,
        user_info.platform
    );

    // 返回成功响应
    Ok(success_response(
        serde_json::json!({}),
        StatusCode::OK,
    ))
}

/// 批量获取好友在线状态请求
#[derive(Debug, Deserialize)]
pub struct BatchGetFriendsOnlineRequest {
    /// 好友用户ID列表
    pub user_ids: Vec<String>,
}

/// 批量获取好友在线状态响应
#[derive(Debug, Serialize)]
pub struct BatchGetFriendsOnlineResponse {
    /// 好友在线状态列表
    pub friends_status: Vec<cache::UserOnlineStatus>,
    /// 请求的用户数量
    pub total_count: usize,
    /// 在线用户数量
    pub online_count: usize,
}

/// 批量获取好友在线状态
///
/// 支持一次性查询多个用户的在线状态信息，包括全局在线状态和各平台在线情况
pub async fn batch_get_friends_online_status(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    Extension(user_info): Extension<Claims>,
    Json(request): Json<BatchGetFriendsOnlineRequest>,
) -> Result<impl IntoResponse, Error> {
    let requester_user_id = user_info.sub;

    // 验证请求参数
    if request.user_ids.is_empty() {
        return Ok(error_response(
            "用户ID列表不能为空",
            StatusCode::BAD_REQUEST,
        ));
    }

    // 去重用户ID
    let mut unique_user_ids: Vec<String> = request.user_ids.clone();
    unique_user_ids.sort();
    unique_user_ids.dedup();

    // 批量获取用户在线状态
    let friends_status = match cache_instance
        .batch_get_users_online_status(&unique_user_ids)
        .await
    {
        Ok(status_list) => status_list,
        Err(e) => {
            error!("批量获取用户在线状态失败: {}", e);
            return Ok(error_response(
                "获取好友在线状态失败",
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    // 统计在线用户数量
    let online_count = friends_status
        .iter()
        .filter(|status| status.is_online)
        .count();
    let total_count = friends_status.len();

    info!(
        "用户{}成功查询{}个好友的在线状态，其中{}个在线",
        requester_user_id, total_count, online_count
    );

    // 构建响应
    let response = BatchGetFriendsOnlineResponse {
        friends_status,
        total_count,
        online_count,
    };

    Ok(success_response(response, StatusCode::OK))
}
