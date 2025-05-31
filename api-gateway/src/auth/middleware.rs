use axum::{
    extract::{Extension, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use common::config::ConfigLoader;
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::auth::controller::PlatformType;
use crate::auth::jwt;
use crate::middleware::get_client_ip;

/// 认证中间件
pub async fn auth_middleware(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    mut req: Request,
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
            warn!(
                "JWT认证失败: 路径={}, IP={}, 原因=缺少认证令牌",
                path, client_ip
            );
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
    let user_info = match jwt::verify_token(&token, jwt_config) {
        Ok(user_info) => user_info,
        Err(e) => {
            warn!("JWT认证失败: 路径={}, IP={}, 原因={}", path, client_ip, e);
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "message": "无效的认证令牌"
                })),
            )
                .into_response();
        }
    };

    // 从JWT token中提取平台信息，而不是从请求头
    let platform = user_info.extra
        .get("platform")
        .map(|platform_str| PlatformType::from(platform_str.as_str()))
        .unwrap_or(PlatformType::Unknown);
    let platform_str = platform.as_str();

    debug!(
        "认证中间件: 用户ID={}, 平台={}",
        user_info.user_id, platform_str
    );

    // 验证Redis中的token是否存在且匹配
    let user_id_str = user_info.user_id.to_string();
    match cache_instance
        .get_access_token_for_platform(&user_id_str, platform_str)
        .await
    {
        Ok(Some(stored_token)) => {
            if stored_token != token {
                warn!(
                    "Token校验失败: 用户ID={}, 平台={}, 路径={}, IP={}, 原因=Redis中的token与请求token不匹配",
                    user_info.user_id, platform_str, path, client_ip
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": "token_mismatch",
                        "message": "令牌已失效，请重新登录"
                    })),
                )
                    .into_response();
            }
        }
        Ok(None) => {
            warn!(
                "Token校验失败: 用户ID={}, 平台={}, 路径={}, IP={}, 原因=Redis中未找到有效token",
                user_info.user_id, platform_str, path, client_ip
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "token_not_found",
                    "message": "令牌已过期或已注销，请重新登录"
                })),
            )
                .into_response();
        }
        Err(e) => {
            warn!(
                "Redis token校验错误: 用户ID={}, 平台={}, 路径={}, IP={}, 错误={}",
                user_info.user_id, platform_str, path, client_ip, e
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "token_validation_error",
                    "message": "令牌验证服务错误"
                })),
            )
                .into_response();
        }
    }

    debug!(
        "认证成功: 用户ID={}, 用户名={}, 平台={}, 路径={}",
        user_info.user_id, user_info.username, platform_str, path
    );

    // 将用户信息添加到请求扩展中
    req.extensions_mut().insert(user_info);
    next.run(req).await
}
