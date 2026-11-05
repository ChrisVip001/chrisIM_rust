//! 认证中间件
//!
//! 顺序：IP 黑名单 → 路径白名单 → JWT 验签 → Redis 会话比对 → 注入 `Claims`。
//! 任何一步失败都返回带语义的状态码（401/403/503），响应体不含内部错误细节。

use cache::SessionCache;
use axum::{
    extract::{Extension, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use runtime::auth::jwt;
use runtime::config::AppConfig;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::api_utils::net::resolve_client_ip;
use crate::api_utils::path_under;
use crate::error::GatewayError;

/// 认证中间件
pub async fn auth_middleware(
    State(config): State<Arc<AppConfig>>,
    Extension(cache_instance): Extension<Arc<dyn SessionCache>>,
    req: Request,
    next: Next,
) -> Response {
    let auth_config = &config.gateway.auth;
    let path = req.uri().path().to_string();

    // 1. IP 黑名单：命中即 403
    if !auth_config.ip_blacklist.is_empty() {
        if let Some(ip) = resolve_client_ip(&req, &auth_config.trusted_proxies) {
            if auth_config
                .ip_blacklist
                .iter()
                .any(|blocked| {
                    blocked
                        .parse::<std::net::IpAddr>()
                        .map(|b| b == ip)
                        .unwrap_or(false)
                })
            {
                warn!(%ip, path = %path, "命中 IP 黑名单");
                return GatewayError::forbidden("访问被拒绝").into_response();
            }
        }
    }

    // 2. 路径白名单（按路径段边界匹配）
    if auth_config
        .path_whitelist
        .iter()
        .any(|prefix| path_under(&path, prefix))
    {
        return next.run(req).await;
    }

    let jwt_config = &auth_config.jwt;

    // 3. 提取并校验 JWT
    let Some(token) = jwt::extract_token(&req, &jwt_config.header_name, &jwt_config.header_prefix)
    else {
        warn!(path = %path, "未找到认证令牌");
        return GatewayError::unauthorized("缺少认证令牌").into_response();
    };

    let user_info = match jwt::verify_token(&token, jwt_config) {
        Ok(info) => info,
        Err(e) => {
            // 验签失败原因（过期/签名不符）只进日志
            debug!(path = %path, "令牌校验失败: {:?}", e);
            return GatewayError::unauthorized("令牌无效或已过期，请重新登录").into_response();
        }
    };

    // 4. 与 Redis 中该平台的会话比对（支持「同平台重登踢掉旧 token」）
    match cache_instance
        .get_access_token_for_platform(&user_info.sub, user_info.platform)
        .await
    {
        Ok(Some(stored_token)) if stored_token == token => {}
        Ok(_) => {
            return GatewayError::unauthorized("令牌已过期或已注销，请重新登录").into_response();
        }
        Err(e) => {
            warn!(sub = %user_info.sub, "读取会话失败: {:?}", e);
            return GatewayError::unavailable(format!("会话存储不可用: {e}")).into_response();
        }
    }

    debug!(
        "认证成功: 用户ID={}, 用户名={}, 平台={}, 路径={}",
        user_info.sub, user_info.username, user_info.platform, path
    );

    let mut request = req;
    request.extensions_mut().insert(user_info);
    next.run(request).await
}
