pub mod controller;
pub mod jwt;
pub mod middleware;

use crate::middleware::get_client_ip;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use common::config::ConfigLoader;
use common::error::Error;

/// 统一认证入口
pub async fn authenticate(
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Error> {
    let config = &ConfigLoader::get_global()
        .expect("Failed to get global config")
        .gateway;

    // 检查路径是否在白名单中
    let path = request.uri().path().to_string();
    if config
        .auth
        .path_whitelist
        .iter()
        .any(|p| path.starts_with(p))
    {
        // 白名单路径，直接放行
        return Ok(next.run(request).await);
    }

    // 检查IP是否在白名单中
    let client_ip = get_client_ip(&request);
    if config.auth.ip_whitelist.contains(&client_ip) {
        // IP白名单，直接放行
        return Ok(next.run(request).await);
    }

    // 获取JWT token并验证
    let jwt_config = &config.auth.jwt;
    let token =
        match jwt::extract_token(&request, &jwt_config.header_name, &jwt_config.header_prefix) {
            Some(token) => token,
            None => return Err(Error::Unauthorized),
        };

    // 解析和验证token
    let user_info = match jwt::verify_token(token, jwt_config).await {
        Ok(info) => info,
        Err(err) => return Err(err),
    };

    // 添加用户信息到请求中
    let mut request = request;
    request.extensions_mut().insert(user_info);

    Ok(next.run(request).await)
}
