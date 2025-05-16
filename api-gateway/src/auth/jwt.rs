use axum::http::Request;
use common::error::Error;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// 用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// 用户ID
    pub user_id: i64,
    /// 用户名
    pub username: String,
    /// 租户ID
    pub tenant_id: i64,
    /// 租户名称
    pub tenant_name: String,
    /// 额外信息
    #[serde(default)]
    pub extra: std::collections::HashMap<String, String>,
}

/// JWT Token中的声明信息
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// 主题 (用户ID)
    pub sub: String,
    /// 签发者
    pub iss: Option<String>,
    /// 过期时间
    pub exp: u64,
    /// 签发时间
    pub iat: u64,
    /// 用户名
    pub username: String,
    /// 租户ID
    pub tenant_id: i64,
    /// 租户名称
    pub tenant_name: String,
    /// 额外信息
    #[serde(default)]
    pub extra: std::collections::HashMap<String, String>,
}

/// 从请求头中提取token
pub fn extract_token<B>(
    request: &Request<B>,
    header_name: &str,
    header_prefix: &str,
) -> Option<String> {
    request
        .headers()
        .get(header_name)
        .and_then(|value| value.to_str().ok())
        .and_then(|auth_header| {
            if auth_header.starts_with(header_prefix) {
                Some(auth_header[header_prefix.len()..].to_string())
            } else {
                None
            }
        })
}

/// 验证JWT Token
pub async fn verify_token(
    token: String,
    jwt_config: &crate::config::auth_config::JwtConfig,
) -> Result<UserInfo, Error> {
    // 解码并验证token
    let mut validation = Validation::new(Algorithm::HS256);
    if jwt_config.verify_issuer && !jwt_config.allowed_issuers.is_empty() {
        validation.iss = Some(jwt_config.allowed_issuers.clone().into_iter().collect());
    }

    let token_data = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(jwt_config.secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => Error::TokenExpired,
        jsonwebtoken::errors::ErrorKind::InvalidIssuer => Error::InvalidIssuer,
        _ => Error::InvalidToken,
    })?;

    // 检查token是否过期
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Internal(e.to_string()))?
        .as_secs();

    if token_data.claims.exp <= now {
        return Err(Error::TokenExpired);
    }

    // 构建用户信息
    let user_info = UserInfo {
        user_id: token_data
            .claims
            .sub
            .parse::<i64>()
            .map_err(|_| Error::InvalidToken)?,
        username: token_data.claims.username,
        tenant_id: token_data.claims.tenant_id,
        tenant_name: token_data.claims.tenant_name,
        extra: token_data.claims.extra,
    };

    Ok(user_info)
}

/// 生成JWT Token
pub fn generate_token(
    user_id: i64,
    username: &str,
    tenant_id: i64,
    tenant_name: &str,
    extra: std::collections::HashMap<String, String>,
    jwt_config: &crate::config::auth_config::JwtConfig,
) -> Result<String, Error> {
    // 获取当前时间戳
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Internal(e.to_string()))?
        .as_secs();

    // 创建Claims
    let claims = Claims {
        sub: user_id.to_string(),
        iss: Some(jwt_config.issuer.clone()),
        exp: now + jwt_config.expiry_seconds,
        iat: now,
        username: username.to_string(),
        tenant_id,
        tenant_name: tenant_name.to_string(),
        extra,
    };

    // 生成token
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(jwt_config.secret.as_bytes()),
    )
    .map_err(|e| Error::Internal(format!("生成JWT令牌失败: {}", e)))?;

    Ok(token)
}

/// 生成刷新Token
pub fn generate_refresh_token(
    user_id: i64,
    username: &str,
    tenant_id: i64,
    tenant_name: &str,
    jwt_config: &crate::config::auth_config::JwtConfig,
) -> Result<String, Error> {
    // 获取当前时间戳
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Internal(e.to_string()))?
        .as_secs();

    // 创建Claims (刷新令牌通常不包含太多信息)
    let claims = Claims {
        sub: user_id.to_string(),
        iss: Some(jwt_config.issuer.clone()),
        exp: now + jwt_config.refresh_expiry_seconds,
        iat: now,
        username: username.to_string(),
        tenant_id,
        tenant_name: tenant_name.to_string(),
        extra: std::collections::HashMap::new(),
    };

    // 生成token
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(jwt_config.secret.as_bytes()),
    )
    .map_err(|e| Error::Internal(format!("生成刷新令牌失败: {}", e)))?;

    Ok(token)
}
