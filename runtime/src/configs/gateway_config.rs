use serde::{Deserialize, Serialize};
use crate::configs::auth_config::AuthConfig;
use crate::configs::rate_limit_config::RateLimitConfig;
use crate::configs::routes_config::RoutesConfig;

/// 网关配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// 路由配置
    pub routes: RoutesConfig,
    /// 限流配置
    pub rate_limit: RateLimitConfig,
    /// 认证配置
    pub auth: AuthConfig,
    /// Metrics暴露端点
    pub metrics_endpoint: String,
    /// 重试配置
    pub retry: RetryConfig,
    /// 熔断配置
    pub circuit_breaker: CircuitBreakerConfig,
    /// CORS 配置
    #[serde(default)]
    pub cors: CorsConfig,
}

/// 跨域配置
///
/// 来源由配置决定，默认空 = 不放行任何跨域来源。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorsConfig {
    /// 允许的来源，如 `["https://im.example.com"]`
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// 是否允许携带凭证
    #[serde(default)]
    pub allow_credentials: bool,
}

/// 重试配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// 最大重试次数
    pub max_retries: usize,
    /// 重试间隔（毫秒）
    pub retry_interval_ms: u64,
}

/// 熔断配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// 开启熔断功能
    pub enabled: bool,
    /// 熔断失败阈值
    pub failure_threshold: u64,
    /// 半开状态超时时间（秒）
    pub half_open_timeout_secs: u64,
    /// 半开态同时放行的探测请求数：超出的请求快速失败，不再打下游。
    /// 缺省 3，旧配置文件不写该字段也能启动。
    #[serde(default = "default_half_open_max_probes")]
    pub half_open_max_probes: u32,
    /// 半开态需要连续成功的次数，达到后关闭熔断器。缺省 2。
    #[serde(default = "default_half_open_success_threshold")]
    pub half_open_success_threshold: u32,
}

fn default_half_open_max_probes() -> u32 {
    3
}

fn default_half_open_success_threshold() -> u32 {
    2
}

