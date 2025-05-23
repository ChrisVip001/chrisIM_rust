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
}

