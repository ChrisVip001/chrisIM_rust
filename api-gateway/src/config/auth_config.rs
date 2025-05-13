use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// 认证配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// JWT配置
    pub jwt: JwtConfig,
    /// API Key配置
    pub api_key: ApiKeyConfig,
    /// OAuth2配置
    pub oauth2: OAuth2Config,
    /// IP白名单
    #[serde(default)]
    pub ip_whitelist: Vec<String>,
    /// 路径白名单（不需要认证的路径）
    #[serde(default)]
    pub path_whitelist: Vec<String>,
}

/// JWT配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    /// 是否启用JWT认证
    pub enabled: bool,
    /// JWT密钥
    pub secret: String,
    /// 签发者
    pub issuer: String,
    /// 过期时间（秒）
    pub expiry_seconds: u64,
    /// 刷新令牌过期时间（秒）
    pub refresh_expiry_seconds: u64,
    /// 是否检查签发者
    pub verify_issuer: bool,
    /// 允许的签发者列表
    #[serde(default)]
    pub allowed_issuers: Vec<String>,
    /// 认证头名称
    pub header_name: String,
    /// 认证头前缀
    pub header_prefix: String,
}

/// API Key配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConfig {
    /// 是否启用API Key认证
    pub enabled: bool,
    /// API Key头名称
    pub header_name: String,
    /// 有效的API Key列表
    #[serde(default)]
    pub api_keys: HashMap<String, ApiKeyInfo>,
}

/// API Key信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    /// 密钥名称
    pub name: String,
    /// 关联用户ID
    pub user_id: Option<i64>,
    /// 权限列表
    #[serde(default)]
    pub permissions: Vec<String>,
    /// 是否启用
    pub enabled: bool,
    /// 到期时间（ISO 8601格式，如2023-12-31T23:59:59Z）
    pub expires_at: Option<String>,
}

/// OAuth2配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    /// 是否启用OAuth2认证
    pub enabled: bool,
    /// 客户端ID
    pub client_id: String,
    /// 客户端密钥
    pub client_secret: String,
    /// 认证端点
    pub auth_url: String,
    /// 令牌端点
    pub token_url: String,
    /// 重定向URL
    pub redirect_url: String,
    /// 范围
    pub scope: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt: JwtConfig {
                enabled: true,
                secret: "change_this_to_a_secure_random_string".to_string(),
                issuer: "api-gateway".to_string(),
                expiry_seconds: 86400, // 24小时
                refresh_expiry_seconds: 604800, // 7天
                verify_issuer: false,
                allowed_issuers: vec![],
                header_name: "Authorization".to_string(),
                header_prefix: "Bearer ".to_string(),
            },
            api_key: ApiKeyConfig {
                enabled: false,
                header_name: "X-API-Key".to_string(),
                api_keys: HashMap::new(),
            },
            oauth2: OAuth2Config {
                enabled: false,
                client_id: "".to_string(),
                client_secret: "".to_string(),
                auth_url: "".to_string(),
                token_url: "".to_string(),
                redirect_url: "".to_string(),
                scope: "".to_string(),
            },
            ip_whitelist: vec![
                "127.0.0.1".to_string(),
                "::1".to_string(),
            ],
            path_whitelist: vec![
                "/api/health".to_string(),
                "/api/auth/login".to_string(),
                "/api/auth/register".to_string(),
                "/metrics".to_string(),
            ],
        }
    }
} 