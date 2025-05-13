use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// 限流配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// 全局限流配置
    pub global: RateLimitRule,
    /// 按路径限流配置
    pub path_rules: Vec<PathRateLimitRule>,
    /// 按API密钥限流配置
    pub api_key_rules: HashMap<String, RateLimitRule>,
    /// 按IP限流配置
    pub ip_rules: HashMap<String, RateLimitRule>,
}

/// 按路径限流规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRateLimitRule {
    /// 路径前缀
    pub path_prefix: String,
    /// 限流规则
    pub rule: RateLimitRule,
}

/// 限流规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRule {
    /// 每秒请求数
    pub requests_per_second: u32,
    /// 突发请求数
    pub burst_size: u32,
    /// 是否启用
    pub enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            global: RateLimitRule {
                requests_per_second: 1000,
                burst_size: 50,
                enabled: true,
            },
            path_rules: vec![
                // 默认限流规则 - 登录接口
                PathRateLimitRule {
                    path_prefix: "/api/auth/login".to_string(),
                    rule: RateLimitRule {
                        requests_per_second: 5,
                        burst_size: 3,
                        enabled: true,
                    },
                },
                // 默认限流规则 - 注册接口
                PathRateLimitRule {
                    path_prefix: "/api/auth/register".to_string(),
                    rule: RateLimitRule {
                        requests_per_second: 2,
                        burst_size: 5,
                        enabled: true,
                    },
                },
                // 默认限流规则 - 用户接口
                PathRateLimitRule {
                    path_prefix: "/api/users".to_string(),
                    rule: RateLimitRule {
                        requests_per_second: 10,
                        burst_size: 20,
                        enabled: true,
                    },
                },
            ],
            api_key_rules: HashMap::new(),
            ip_rules: HashMap::new(),
        }
    }
} 