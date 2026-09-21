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
use tracing::{debug, warn, error};
use common::auth::jwt;
use crate::middleware::get_client_ip;
use crate::proxy::services::common::{
    error_response,success_response
};

/// 认证中间件
pub async fn auth_middleware(
    Extension(cache_instance): Extension<Arc<dyn cache::Cache>>,
    req: Request,
    next: Next,
) -> Response {
    let config = match ConfigLoader::get_global() {
        Some(config) => config,
        None => return error_response("获取配置错误！", StatusCode::INTERNAL_SERVER_ERROR),
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

    // TODO 检查IP是否在黑名单中
    let client_ip = get_client_ip(&req);

    // 获取JWT配置
    let jwt_config = &config.gateway.auth.jwt;

    // 使用jwt::extract_token函数提取token
    let token = match jwt::extract_token(&req, &jwt_config.header_name, &jwt_config.header_prefix) {
        Some(token) => token,
        None => {
            warn!("未找到认证令牌，请求路径: {}", path);
            return error_response("缺少认证令牌", StatusCode::UNAUTHORIZED);
        }
    };

    // 验证JWT token
    let user_info = match jwt::verify_token(&token, jwt_config) {
        Ok(user_info) => user_info,
        Err(e) => {
            return error_response(&format!("令牌验证失败: {:?}", e), StatusCode::UNAUTHORIZED)
        }
    };

    // 从JWT token中提取平台信息，而不是从请求头
    let platform = user_info.platform;

    // 验证Redis中的token是否存在且匹配
    match cache_instance
        .get_access_token_for_platform(&user_info.sub, platform)
        .await
    {
        Ok(Some(stored_token)) => {
            if stored_token != token {
                return error_response("令牌已过期或已注销，请重新登录", StatusCode::UNAUTHORIZED);
            }
        }
        Ok(None) => {
            return error_response("令牌已过期或已注销，请重新登录", StatusCode::UNAUTHORIZED);
        }
        Err(e) => {
            error!("Redis查询失败: {:?}", e);
            return error_response(&format!("令牌验证服务错误: {:?}", e), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    debug!(
        "认证成功: 用户ID={}, 用户名={}, 平台={}, 路径={}",
        user_info.sub, user_info.username, platform, path
    );

    // 将用户信息和平台信息添加到请求扩展中
    let mut request = req;
    request.extensions_mut().insert(user_info);
    request.extensions_mut().insert(platform);
    next.run(request).await
}
