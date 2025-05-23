use serde::{Deserialize, Serialize};

/// 认证配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// JWT配置
    pub jwt: JwtConfig,
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

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt: JwtConfig {
                secret: "change_this_to_a_secure_random_string".to_string(),
                issuer: "api-gateway".to_string(),
                expiry_seconds: 86400,          // 24小时
                refresh_expiry_seconds: 604800, // 7天
                verify_issuer: false,
                allowed_issuers: vec![],
                header_name: "Authorization".to_string(),
                header_prefix: "Bearer ".to_string(),
            },
            ip_whitelist: vec!["127.0.0.1".to_string(), "::1".to_string()],
            path_whitelist: vec![
                "/api/health".to_string(),
                "/api/auth/login".to_string(),
                "/api/auth/register".to_string(),
                "/metrics".to_string(),
            ],
        }
    }
}
