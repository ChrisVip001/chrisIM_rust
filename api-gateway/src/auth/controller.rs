use crate::auth::jwt;
use crate::proxy::services::common::{error_response, success_response};
use axum::extract::Extension;
use axum::http::StatusCode;
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
use tracing::{debug, error, info};

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
    Json(login_req): Json<LoginByPhoneRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("短信验证码登录请求：手机号 {}", login_req.phone);

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

    info!("用户 {} 手机号登录成功", user.username);

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
    )
    .await?;

    // 返回响应
    Ok(success_response(login_response, StatusCode::OK))
}

/// 处理登录请求
pub async fn login(
    Extension(user_service): Extension<SharedUserService>,
    Json(login_req): Json<LoginRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("登录请求：用户 {}", login_req.username);

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
        return Err(Error::Authentication("用户名或密码不正确".to_string()));
    }

    // 获取用户信息
    let user = response.user.unwrap();

    info!("用户 {} 登录成功", login_req.username);

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
    )
    .await?;

    // 返回响应
    Ok(success_response(login_response, StatusCode::OK))
}

/// 处理令牌刷新请求
pub async fn refresh_token(
    Json(refresh_req): Json<RefreshTokenRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("刷新令牌请求");

    // 读取JWT配置
    let config = ConfigLoader::get_global().expect("Failed to get global config");

    let jwt_config = &config.gateway.auth.jwt;

    // 验证刷新令牌
    let user_info = jwt::verify_token(refresh_req.refresh_token, jwt_config).await?;

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
    )
    .await?;

    info!("用户 {} 刷新令牌成功", username);

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
) -> Result<LoginResponse, Error> {
    // 读取JWT配置
    let config = ConfigLoader::get_global().expect("Failed to get global config");

    let jwt_config = &config.gateway.auth.jwt;

    // 生成访问令牌
    let access_token = jwt::generate_token(
        user_id,
        username,
        tenant_id,
        tenant_name,
        extra.clone(),
        jwt_config,
    )?;

    // 生成刷新令牌
    let refresh_token =
        jwt::generate_refresh_token(user_id, username, tenant_id, tenant_name, jwt_config)?;

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
