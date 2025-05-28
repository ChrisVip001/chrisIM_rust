use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use common::config::ConfigLoader;
use serde_json::json;
use tracing::warn;

use crate::auth::jwt;
use crate::middleware::get_client_ip;

/// 认证中间件
pub async fn auth_middleware(req: Request, next: Next) -> Response {
    let config = match ConfigLoader::get_global() {
        Some(config) => config,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "config_error",
                    "message": "服务器配置错误"
                })),
            )
                .into_response();
        }
    };

    // 检查路径是否在白名单中
    let path = req.uri().path().to_string();
    if config
        .gateway
        .auth
        .path_whitelist
        .iter()
        .any(|p| path.starts_with(p))
    {
        // 白名单路径，直接放行
        return next.run(req).await;
    }

    // 检查IP是否在白名单中
    let client_ip = get_client_ip(&req);
    if config.gateway.auth.ip_whitelist.contains(&client_ip) {
        // IP白名单，直接放行
        return next.run(req).await;
    }

    // 获取JWT配置
    let jwt_config = &config.gateway.auth.jwt;

    // 使用jwt::extract_token函数提取token
    let token = match jwt::extract_token(&req, &jwt_config.header_name, &jwt_config.header_prefix) {
        Some(token) => token,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "message": "缺少认证令牌"
                })),
            )
                .into_response();
        }
    };

    // 验证JWT token
    match jwt::verify_token(&token, jwt_config) {
        Ok(user_info) => {
            // 将用户信息添加到请求扩展中
            let mut request = req;
            request.extensions_mut().insert(user_info);
            next.run(request).await
        }
        Err(e) => {
            warn!("JWT验证失败: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "message": "无效的认证令牌"
                })),
            )
                .into_response()
        }
    }
}
