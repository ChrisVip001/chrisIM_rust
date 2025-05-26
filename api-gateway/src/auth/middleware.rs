use axum::{
    extract::Request,
    http::{StatusCode, HeaderMap},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use common::config::ConfigLoader;
use serde_json::json;
use tracing::warn;

use crate::auth::jwt;

/// 认证中间件
pub async fn auth_middleware(
    req: Request,
    next: Next,
) -> Response {
    let config = match ConfigLoader::get_global() {
        Some(config) => config,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "config_error",
                    "message": "服务器配置错误"
                })),
            ).into_response();
        }
    };

    // 手动提取 Authorization header
    let auth_header = req.headers().get("authorization");
    let token = match auth_header {
        Some(header_value) => {
            match header_value.to_str() {
                Ok(header_str) => {
                    if header_str.starts_with("Bearer ") {
                        header_str.strip_prefix("Bearer ").unwrap_or("").to_string()
                    } else {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({
                                "error": "unauthorized",
                                "message": "无效的认证令牌格式"
                            })),
                        ).into_response();
                    }
                }
                Err(_) => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({
                            "error": "unauthorized",
                            "message": "无效的认证令牌格式"
                        })),
                    ).into_response();
                }
            }
        }
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "message": "缺少认证令牌"
                })),
            ).into_response();
        }
    };

    let jwt_config = &config.gateway.auth.jwt;

    // 验证JWT token
    match jwt::verify_token(token, jwt_config).await {
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
            ).into_response()
        }
    }
}
