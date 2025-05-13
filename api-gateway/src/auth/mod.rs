pub mod jwt;
pub mod api_key;
pub mod oauth2;
pub mod middleware;

use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use axum::body::Bytes;
use crate::config::CONFIG;
use common::error::Error;

/// 统一认证入口
pub async fn authenticate(request: Request<axum::body::Body>, next: Next) -> Result<Response, Error>
{
    let config = CONFIG.read().await;
    
    // 检查路径是否在白名单中
    let path = request.uri().path().to_string();
    if config.auth.path_whitelist.iter().any(|p| path.starts_with(p)) {
        // 白名单路径，直接放行
        return Ok(next.run(request).await);
    }
    
    // 检查IP是否在白名单中
    let client_ip = get_client_ip(&request);
    if let Some(ip) = client_ip {
        if config.auth.ip_whitelist.contains(&ip) {
            // IP白名单，直接放行
            return Ok(next.run(request).await);
        }
    }
    
    // 使用对应的认证方式
    if config.auth.jwt.enabled {
        // 获取JWT token并验证
        let jwt_config = &config.auth.jwt;
        let token = match jwt::extract_token(&request, &jwt_config.header_name, &jwt_config.header_prefix) {
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
        
        return Ok(next.run(request).await);
    } else if config.auth.api_key.enabled {
        // 从headers中获取API key
        let api_key_config = &config.auth.api_key;
        let api_key = match request.headers().get(&api_key_config.header_name).and_then(|v| v.to_str().ok()) {
            Some(key) => key.to_string(),
            None => return Err(Error::InvalidApiKey),
        };
        
        // 验证API key
        let api_key_info = match api_key_config.api_keys.get(&api_key) {
            Some(info) => info,
            None => return Err(Error::InvalidApiKey),
        };
        
        // 检查API key有效性
        if !api_key_info.enabled {
            return Err(Error::InvalidApiKey);
        }
        
        // 检查是否过期
        if let Some(expires_at) = &api_key_info.expires_at {
            match chrono::DateTime::parse_from_rfc3339(expires_at) {
                Ok(expiry_time) => {
                    if expiry_time < chrono::Utc::now() {
                        return Err(Error::ApiKeyExpired);
                    }
                },
                Err(_) => {
                    return Err(Error::Internal("无效的API Key过期时间格式".to_string()));
                }
            }
        }
        
        // 获取用户ID
        let user_id = match api_key_info.user_id {
            Some(id) => id,
            None => return Err(Error::Internal("API Key未关联用户ID".to_string())),
        };
        
        // 构建用户信息
        let user_info = jwt::UserInfo {
            user_id,
            username: api_key_info.name.clone(),
            roles: api_key_info.permissions.clone(),
            extra: Default::default(),
        };
        
        // 添加用户信息到请求中
        let mut request = request;
        request.extensions_mut().insert(user_info);
        
        return Ok(next.run(request).await);
    } else if config.auth.oauth2.enabled {
        // OAuth2认证逻辑
        let token = match oauth2::extract_oauth_token(&request) {
            Some(t) => t,
            None => return Err(Error::Unauthorized),
        };
        
        // 验证token (简化实现)
        // 实际应用中应调用OAuth2提供商的API验证token
        // 这里仅作示例
        
        // 构建模拟用户信息
        let user_info = jwt::UserInfo {
            user_id: 10000, // 从OAuth提供商获取
            username: "oauth_user".to_string(),
            roles: vec!["user".to_string()],
            extra: Default::default(),
        };
        
        // 添加用户信息到请求中
        let mut request = request;
        request.extensions_mut().insert(user_info);
        
        return Ok(next.run(request).await);
    } else {
        // 如果没有启用任何认证方式，返回未授权错误
        return Err(Error::Unauthorized);
    }
}

/// 从请求中获取客户端IP
fn get_client_ip<B>(request: &Request<B>) -> Option<String> {
    request.headers()
        .get("X-Forwarded-For")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or("").trim().to_string())
        .or_else(|| {
            request.headers()
                .get("X-Real-IP")
                .and_then(|value| value.to_str().ok())
                .map(|s| s.to_string())
        })
} 