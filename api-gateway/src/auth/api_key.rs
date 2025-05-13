use axum::http::Request;
use common::error::Error;
use crate::auth::jwt::UserInfo;
use crate::config::CONFIG;
use chrono::{DateTime, Utc};

/// 通过API Key进行认证
pub async fn authenticate_api_key<B>(request: &Request<B>) -> Result<UserInfo, Error> {
    let config = CONFIG.read().await;
    let api_key_config = &config.auth.api_key;
    
    // 从请求头中提取API Key
    let api_key = request.headers()
        .get(&api_key_config.header_name)
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(Error::InvalidApiKey)?;
    
    // 查找API Key
    let api_key_info = api_key_config.api_keys.get(&api_key)
        .ok_or(Error::InvalidApiKey)?;
    
    // 检查API Key是否启用
    if !api_key_info.enabled {
        return Err(Error::InvalidApiKey);
    }
    
    // 检查API Key是否过期
    if let Some(expires_at) = &api_key_info.expires_at {
        match DateTime::parse_from_rfc3339(expires_at) {
            Ok(expiry_time) => {
                if expiry_time < Utc::now() {
                    return Err(Error::ApiKeyExpired);
                }
            },
            Err(_) => {
                return Err(Error::Internal("无效的API Key过期时间格式".to_string()));
            }
        }
    }
    
    // 获取用户ID
    let user_id = api_key_info.user_id
        .ok_or(Error::Internal("API Key未关联用户ID".to_string()))?;
    
    // 构建用户信息
    let user_info = UserInfo {
        user_id,
        username: api_key_info.name.clone(),
        roles: api_key_info.permissions.clone(),
        extra: Default::default(),
    };
    
    Ok(user_info)
}

/// 通过API Key进行认证（拥有请求所有权版本）
pub async fn authenticate_api_key_owned<B>(request: Request<B>) -> Result<(Request<B>, UserInfo), (Request<B>, Error)>
where 
    B: axum::body::HttpBody + Send + 'static,
    B::Error: std::fmt::Display + Send + Sync + 'static
{
    let config = CONFIG.read().await;
    let api_key_config = &config.auth.api_key;
    
    // 从请求头中提取API Key
    let api_key = match request.headers()
        .get(&api_key_config.header_name)
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(key) => key,
        None => return Err((request, Error::InvalidApiKey)),
    };
    
    // 查找API Key
    let api_key_info = match api_key_config.api_keys.get(&api_key) {
        Some(info) => info,
        None => return Err((request, Error::InvalidApiKey)),
    };
    
    // 检查API Key是否启用
    if !api_key_info.enabled {
        return Err((request, Error::InvalidApiKey));
    }
    
    // 检查API Key是否过期
    if let Some(expires_at) = &api_key_info.expires_at {
        match DateTime::parse_from_rfc3339(expires_at) {
            Ok(expiry_time) => {
                if expiry_time < Utc::now() {
                    return Err((request, Error::ApiKeyExpired));
                }
            },
            Err(_) => {
                return Err((request, Error::Internal("无效的API Key过期时间格式".to_string())));
            }
        }
    }
    
    // 获取用户ID
    let user_id = match api_key_info.user_id {
        Some(id) => id,
        None => return Err((request, Error::Internal("API Key未关联用户ID".to_string()))),
    };
    
    // 构建用户信息
    let user_info = UserInfo {
        user_id,
        username: api_key_info.name.clone(),
        roles: api_key_info.permissions.clone(),
        extra: Default::default(),
    };
    
    Ok((request, user_info))
} 