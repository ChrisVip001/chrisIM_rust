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
    /// 安全头配置
    pub security_headers: Option<SecurityHeadersConfig>,
    /// 请求验证配置
    pub request_validation: Option<RequestValidationConfig>,
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

/// 安全头配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityHeadersConfig {
    /// 是否启用安全头
    pub enabled: bool,
    /// 内容安全策略
    pub content_security_policy: String,
    /// X-Frame-Options
    pub x_frame_options: String,
    /// X-Content-Type-Options
    pub x_content_type_options: String,
    /// X-XSS-Protection
    pub x_xss_protection: String,
    /// Strict-Transport-Security
    pub strict_transport_security: String,
    /// Referrer-Policy
    pub referrer_policy: String,
}

/// 请求验证配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestValidationConfig {
    /// 最大请求大小（MB）
    pub max_request_size_mb: u64,
    /// 最大请求头数量
    pub max_header_count: u32,
    /// 单个请求头最大大小（KB）
    pub max_header_size_kb: u64,
    /// 阻止的User-Agent列表
    pub blocked_user_agents: Vec<String>,
    /// IP黑名单
    pub blocked_ips: Vec<String>,
}
