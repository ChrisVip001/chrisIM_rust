use axum::http::Request;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Validation, Algorithm, Header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::error::Error;
use crate::configs::auth_config::JwtConfig;

/// 统一的 JWT Claims 结构
/// 
/// 这个结构在 api-gateway 和 msg-gateway 之间保持一致，
/// 确保 token 的生成和验证逻辑完全相同。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// 主题 (用户ID)
    /// JWT 标准字段，表示令牌的主体用户
    pub sub: String,
    
    /// 签发者
    /// JWT 标准字段，表示令牌的签发者
    pub iss: Option<String>,
    
    /// 过期时间 (Expiration Time)
    /// JWT 标准字段，Unix时间戳格式
    pub exp: u64,
    
    /// 签发时间 (Issued At)
    /// JWT 标准字段，Unix时间戳格式
    pub iat: u64,
    
    /// 用户名
    /// 业务字段，用于用户识别
    pub username: String,
    
    /// 租户ID
    /// 业务字段，用于多租户支持
    pub tenant_id: i64,
    
    /// 租户名称
    /// 业务字段，租户的显示名称
    pub tenant_name: String,
    
    /// 额外信息
    /// 业务字段，存储其他自定义信息
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

/// 用户信息结构
/// 
/// 从 JWT token 中解析出的用户信息
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
    pub extra: HashMap<String, String>,
}

/// 从请求头中提取 JWT token
/// 
/// 从 HTTP 请求头中提取 JWT token，支持自定义头名称和前缀。
/// 通常用于从 Authorization 头中提取 Bearer token。
/// 
/// # 参数
/// * `request` - HTTP 请求对象
/// * `header_name` - 头名称，如 "Authorization"
/// * `header_prefix` - 前缀，如 "Bearer "
/// 
/// # 返回值
/// * `Some(String)` - 成功提取的 token
/// * `None` - 未找到或格式不正确
/// 
/// # 示例
/// ```rust
/// use axum::http::Request;
/// use common::auth::extract_token;
/// 
/// // 从 Authorization: Bearer <token> 中提取 token
/// let token = extract_token(&request, "Authorization", "Bearer ");
/// ```
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

/// 验证 JWT Token
/// 
/// 统一的 token 验证函数，确保 api-gateway 和 msg-gateway 
/// 使用相同的验证逻辑和参数。
/// 
/// # 参数
/// * `token` - JWT token 字符串
/// * `jwt_config` - JWT 配置信息
/// 
/// # 返回值
/// * `Ok(UserInfo)` - 验证成功，返回用户信息
/// * `Err(Error)` - 验证失败
pub fn verify_token(
    token: &str,
    jwt_config: &JwtConfig,
) -> Result<UserInfo, Error> {
    // 设置验证参数
    let mut validation = Validation::new(Algorithm::HS256);
    
    // 配置签发者验证
    if jwt_config.verify_issuer && !jwt_config.allowed_issuers.is_empty() {
        validation.iss = Some(jwt_config.allowed_issuers.clone().into_iter().collect());
    }

    // 解码和验证 token
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_config.secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => Error::TokenExpired,
        jsonwebtoken::errors::ErrorKind::InvalidIssuer => Error::InvalidIssuer,
        _ => Error::InvalidToken,
    })?;

    // 额外的过期时间检查（双重保险）
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

/// 简化的 token 验证函数
/// 
/// 仅验证 token 的有效性，不返回用户信息。
/// 适用于只需要验证 token 有效性的场景，如 WebSocket 连接验证。
/// 
/// # 参数
/// * `token` - JWT token 字符串
/// * `jwt_config` - JWT 配置信息
/// 
/// # 返回值
/// * `Ok(())` - 验证成功
/// * `Err(Error)` - 验证失败
pub fn verify_token_simple(
    token: &str,
    jwt_config: &JwtConfig,
) -> Result<(), Error> {
    verify_token(token, jwt_config).map(|_| ())
}

/// 生成 JWT Token
/// 
/// 生成标准的 JWT access token，包含完整的用户信息。
/// 
/// # 参数
/// * `user_id` - 用户ID
/// * `username` - 用户名
/// * `tenant_id` - 租户ID
/// * `tenant_name` - 租户名称
/// * `extra` - 额外信息
/// * `jwt_config` - JWT 配置
/// 
/// # 返回值
/// * `Ok(String)` - 生成的 token
/// * `Err(Error)` - 生成失败
/// 
/// # 示例
/// ```rust
/// use std::collections::HashMap;
/// use common::auth::generate_token;
/// 
/// let token = generate_token(
///     123,
///     "john_doe",
///     1,
///     "default_tenant",
///     HashMap::new(),
///     &jwt_config
/// )?;
/// ```
pub fn generate_token(
    user_id: i64,
    username: &str,
    tenant_id: i64,
    tenant_name: &str,
    extra: HashMap<String, String>,
    jwt_config: &JwtConfig,
) -> Result<String, Error> {
    // 获取当前时间戳
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Internal(e.to_string()))?
        .as_secs();

    // 创建统一的 Claims 结构
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

/// 生成刷新 Token
/// 
/// 生成用于刷新 access token 的 refresh token，通常具有更长的有效期。
/// 
/// # 参数
/// * `user_id` - 用户ID
/// * `username` - 用户名
/// * `tenant_id` - 租户ID
/// * `tenant_name` - 租户名称
/// * `extra` - 额外信息
/// * `jwt_config` - JWT 配置
/// 
/// # 返回值
/// * `Ok(String)` - 生成的刷新 token
/// * `Err(Error)` - 生成失败
/// 
/// # 示例
/// ```rust
/// use std::collections::HashMap;
/// use common::auth::generate_refresh_token;
/// 
/// let refresh_token = generate_refresh_token(
///     123,
///     "john_doe",
///     1,
///     "default_tenant",
///     HashMap::new(),
///     &jwt_config
/// )?;
/// ```
pub fn generate_refresh_token(
    user_id: i64,
    username: &str,
    tenant_id: i64,
    tenant_name: &str,
    extra: HashMap<String, String>,
    jwt_config: &JwtConfig,
) -> Result<String, Error> {
    // 获取当前时间戳
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Internal(e.to_string()))?
        .as_secs();

    // 创建统一的 Claims 结构 (刷新令牌使用更长的过期时间)
    let claims = Claims {
        sub: user_id.to_string(),
        iss: Some(jwt_config.issuer.clone()),
        exp: now + jwt_config.refresh_expiry_seconds,
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
    .map_err(|e| Error::Internal(format!("生成刷新令牌失败: {}", e)))?;

    Ok(token)
}

/// 从 token 中提取用户ID
/// 
/// 快速从 token 中提取用户ID，不进行完整验证。
/// 注意：这个函数不验证 token 的有效性，仅用于提取信息。
/// 
/// # 参数
/// * `token` - JWT token 字符串
/// * `secret` - JWT 密钥
/// 
/// # 返回值
/// * `Ok(i64)` - 用户ID
/// * `Err(Error)` - 提取失败
pub fn extract_user_id(token: &str, secret: &str) -> Result<i64, Error> {
    let validation = Validation::new(Algorithm::HS256);
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| Error::InvalidToken)?;

    token_data
        .claims
        .sub
        .parse::<i64>()
        .map_err(|_| Error::InvalidToken)
}

/// 从 token 中提取完整的 Claims
/// 
/// 提取 token 中的所有 claims 信息，不进行有效性验证。
/// 主要用于调试和日志记录。
/// 
/// # 参数
/// * `token` - JWT token 字符串
/// * `secret` - JWT 密钥
/// 
/// # 返回值
/// * `Ok(Claims)` - Claims 信息
/// * `Err(Error)` - 提取失败
pub fn extract_claims(token: &str, secret: &str) -> Result<Claims, Error> {
    let validation = Validation::new(Algorithm::HS256);
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| Error::InvalidToken)?;

    Ok(token_data.claims)
}

/// 检查 token 是否即将过期
/// 
/// 检查 token 是否在指定时间内过期，用于提前刷新 token。
/// 
/// # 参数
/// * `token` - JWT token 字符串
/// * `secret` - JWT 密钥
/// * `threshold_seconds` - 提前检查的秒数
/// 
/// # 返回值
/// * `Ok(bool)` - true 表示即将过期，false 表示还有足够时间
/// * `Err(Error)` - 检查失败
pub fn is_token_expiring_soon(
    token: &str, 
    secret: &str, 
    threshold_seconds: u64
) -> Result<bool, Error> {
    let claims = extract_claims(token, secret)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Internal(e.to_string()))?
        .as_secs();
    
    Ok(claims.exp <= now + threshold_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn create_test_config() -> JwtConfig {
        JwtConfig {
            secret: "test_secret_key".to_string(),
            issuer: "test_issuer".to_string(),
            expiry_seconds: 3600,
            refresh_expiry_seconds: 86400,
            verify_issuer: false,
            allowed_issuers: vec![],
            header_name: "Authorization".to_string(),
            header_prefix: "Bearer ".to_string(),
        }
    }

    fn create_test_claims() -> Claims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Claims {
            sub: "123".to_string(),
            iss: Some("test_issuer".to_string()),
            exp: now + 3600,
            iat: now,
            username: "test_user".to_string(),
            tenant_id: 1,
            tenant_name: "test_tenant".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn test_verify_valid_token() {
        let config = create_test_config();
        let claims = create_test_claims();
        
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        ).unwrap();

        let result = verify_token(&token, &config);
        assert!(result.is_ok());
        
        let user_info = result.unwrap();
        assert_eq!(user_info.user_id, 123);
        assert_eq!(user_info.username, "test_user");
    }

    #[test]
    fn test_verify_expired_token() {
        let config = create_test_config();
        let mut claims = create_test_claims();
        claims.exp = 1; // 设置为过期时间

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        ).unwrap();

        let result = verify_token(&token, &config);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::TokenExpired));
    }

    #[test]
    fn test_generate_token() {
        let config = create_test_config();
        let extra = HashMap::new();
        
        let result = generate_token(
            123,
            "test_user",
            1,
            "test_tenant",
            extra,
            &config
        );
        
        assert!(result.is_ok());
        let token = result.unwrap();
        
        // 验证生成的 token 可以被正确解析
        let verify_result = verify_token(&token, &config);
        assert!(verify_result.is_ok());
        
        let user_info = verify_result.unwrap();
        assert_eq!(user_info.user_id, 123);
        assert_eq!(user_info.username, "test_user");
    }

    #[test]
    fn test_generate_refresh_token() {
        let config = create_test_config();
        let extra = HashMap::new();
        
        let result = generate_refresh_token(
            123,
            "test_user",
            1,
            "test_tenant",
            extra,
            &config
        );
        
        assert!(result.is_ok());
        let token = result.unwrap();
        
        // 验证生成的刷新 token 可以被正确解析
        let verify_result = verify_token(&token, &config);
        assert!(verify_result.is_ok());
    }

    #[test]
    fn test_extract_user_id() {
        let config = create_test_config();
        let claims = create_test_claims();
        
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        ).unwrap();

        let result = extract_user_id(&token, &config.secret);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_extract_claims() {
        let config = create_test_config();
        let original_claims = create_test_claims();
        
        let token = encode(
            &Header::new(Algorithm::HS256),
            &original_claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        ).unwrap();

        let result = extract_claims(&token, &config.secret);
        assert!(result.is_ok());
        
        let extracted_claims = result.unwrap();
        assert_eq!(extracted_claims.sub, original_claims.sub);
        assert_eq!(extracted_claims.username, original_claims.username);
    }

    #[test]
    fn test_is_token_expiring_soon() {
        let config = create_test_config();
        let mut claims = create_test_claims();
        
        // 设置 token 在 30 秒后过期
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        claims.exp = now + 30;
        
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        ).unwrap();

        // 检查是否在 60 秒内过期（应该返回 true）
        let result = is_token_expiring_soon(&token, &config.secret, 60);
        assert!(result.is_ok());
        assert!(result.unwrap());
        
        // 检查是否在 10 秒内过期（应该返回 false）
        let result = is_token_expiring_soon(&token, &config.secret, 10);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
} 