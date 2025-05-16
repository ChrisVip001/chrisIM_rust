use crate::auth::jwt;
use crate::config::CONFIG;
use crate::UserServiceGrpcClient;
use axum::{
    extract::State, 
    http::StatusCode, 
    response::IntoResponse, 
    Json
};
use common::error::Error;
use common::proto::user::{VerifyPasswordRequest, VerifyPasswordResponse};
use serde::{Deserialize, Serialize};
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
    pub tenant_id: i64,
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
    pub user_id: i64,
    /// 用户名
    pub username: String,
    /// 租户ID
    pub tenant_id: i64,
    /// 租户名称
    pub tenant_name: String,
    /// 用户邮箱
    pub email: Option<String>,
    /// 用户昵称
    pub nickname: Option<String>,
    /// 头像URL
    pub avatar_url: Option<String>,
}

/// 处理登录请求
pub async fn login(
    axum::extract::Extension(user_client): axum::extract::Extension<Arc<UserServiceGrpcClient>>,
    Json(login_req): Json<LoginRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("登录请求：用户 {}", login_req.username);

    // 创建验证密码请求
    let verify_request = VerifyPasswordRequest {
        username: login_req.username.clone(),
        password: login_req.password,
    };

    // 调用用户服务验证密码
    let response = user_client
        .verify_password(verify_request)
        .await
        .map_err(|e| {
            error!("调用用户服务验证密码失败: {}", e);
            Error::Internal(format!("验证密码服务错误: {}", e))
        })?;

    // 检查密码是否有效
    if !response.valid || response.user.is_none() {
        return Err(Error::Authentication("用户名或密码不正确".to_string()));
    }

    // 获取用户信息
    let user = response.user.unwrap();
    
    // 读取JWT配置
    let config = CONFIG.read().await;
    let jwt_config = &config.auth.jwt;

    // 构建额外信息
    let mut extra = std::collections::HashMap::new();
    
    // email在proto中是String类型，但我们需要考虑其可能为空的情况
    if !user.email.is_empty() {
        extra.insert("email".to_string(), user.email.clone());
    }

    // 将user.id (String类型) 转换为i64
    let user_id = user.id.parse::<i64>().map_err(|_| {
        Error::Internal("无法解析用户ID".to_string())
    })?;

    // 生成访问令牌
    let access_token = jwt::generate_token(
        user_id,
        &user.username,
        // 简化示例，在实际应用中应从用户信息中获取租户ID和名称
        1, // 示例租户ID
        "default", // 示例租户名称
        extra.clone(),
        jwt_config,
    )?;

    // 生成刷新令牌
    let refresh_token = jwt::generate_refresh_token(
        user_id,
        &user.username,
        1, // 示例租户ID
        "default", // 示例租户名称
        jwt_config,
    )?;

    // 构建用户信息响应
    let user_info = UserInfoResponse {
        user_id,
        username: user.username,
        tenant_id: 1, // 示例租户ID
        tenant_name: "default".to_string(), // 示例租户名称
        email: if user.email.is_empty() { None } else { Some(user.email) },
        nickname: user.nickname,
        avatar_url: user.avatar_url,
    };

    // 构建登录响应
    let login_response = LoginResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: jwt_config.expiry_seconds,
        user_info,
    };

    info!("用户 {} 登录成功", login_req.username);

    // 返回响应
    Ok((StatusCode::OK, Json(login_response)))
}

/// 处理令牌刷新请求
pub async fn refresh_token(
    Json(refresh_req): Json<RefreshTokenRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("刷新令牌请求");

    // 读取JWT配置
    let config = CONFIG.read().await;
    let jwt_config = &config.auth.jwt;

    // 验证刷新令牌
    let user_info = jwt::verify_token(refresh_req.refresh_token, jwt_config).await?;

    // 构建额外信息
    let extra = user_info.extra.clone();

    // 获取用户信息用于日志
    let username = user_info.username.clone();

    // 构建用户信息响应
    let user_info_resp = UserInfoResponse {
        user_id: user_info.user_id,
        username: user_info.username,
        tenant_id: user_info.tenant_id,
        tenant_name: user_info.tenant_name,
        email: user_info.extra.get("email").cloned(),
        nickname: user_info.extra.get("nickname").cloned(),
        avatar_url: user_info.extra.get("avatar_url").cloned(),
    };

    // 生成新的访问令牌
    let access_token = jwt::generate_token(
        user_info_resp.user_id,
        &user_info_resp.username,
        user_info_resp.tenant_id,
        &user_info_resp.tenant_name,
        extra,
        jwt_config,
    )?;

    // 生成新的刷新令牌
    let refresh_token = jwt::generate_refresh_token(
        user_info_resp.user_id,
        &user_info_resp.username,
        user_info_resp.tenant_id,
        &user_info_resp.tenant_name,
        jwt_config,
    )?;

    // 构建刷新响应
    let refresh_response = LoginResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: jwt_config.expiry_seconds,
        user_info: user_info_resp,
    };

    info!("用户 {} 刷新令牌成功", username);

    // 返回响应
    Ok((StatusCode::OK, Json(refresh_response)))
} 