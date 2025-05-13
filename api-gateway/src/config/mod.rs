pub mod routes_config;
pub mod rate_limit_config;
pub mod auth_config;

use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use once_cell::sync::Lazy;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, Event};
use std::path::Path;
use tracing::{info, error};
use anyhow::{Result, anyhow};

use self::routes_config::RoutesConfig;
use self::rate_limit_config::RateLimitConfig;
use self::auth_config::AuthConfig;

/// 网关配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// 路由配置
    pub routes: RoutesConfig,
    /// 限流配置
    pub rate_limit: RateLimitConfig,
    /// 认证配置
    pub auth: AuthConfig,
    /// 服务发现配置
    pub consul_url: String,
    /// 服务刷新间隔
    pub service_refresh_interval: u64,
    /// Metrics暴露端点
    pub metrics_endpoint: String,
    /// 链路追踪配置
    pub tracing: TracingConfig,
    /// 重试配置
    pub retry: RetryConfig,
    /// 熔断配置
    pub circuit_breaker: CircuitBreakerConfig,
}

/// 追踪配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracingConfig {
    /// 是否启用OpenTelemetry
    pub enable_opentelemetry: bool,
    /// Jaeger端点
    pub jaeger_endpoint: Option<String>,
    /// 采样率
    pub sampling_ratio: f64,
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

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            routes: RoutesConfig::default(),
            rate_limit: RateLimitConfig::default(),
            auth: AuthConfig::default(),
            consul_url: "http://localhost:8500".to_string(),
            service_refresh_interval: 30,
            metrics_endpoint: "/metrics".to_string(),
            tracing: TracingConfig {
                enable_opentelemetry: false,
                jaeger_endpoint: None,
                sampling_ratio: 0.1,
            },
            retry: RetryConfig {
                max_retries: 3,
                retry_interval_ms: 200,
            },
            circuit_breaker: CircuitBreakerConfig {
                enabled: true,
                failure_threshold: 5,
                half_open_timeout_secs: 30,
            },
        }
    }
}

/// 全局配置管理器
pub static CONFIG: Lazy<Arc<RwLock<GatewayConfig>>> = Lazy::new(|| {
    Arc::new(RwLock::new(GatewayConfig::default()))
});

/// 加载配置
pub async fn load_config(config_path: &str) -> Result<()> {
    let config_path = Path::new(config_path);
    
    // 读取配置文件
    let config_str = std::fs::read_to_string(config_path)?;
    let config: GatewayConfig = if config_path.extension().unwrap_or_default() == "yaml" 
                                || config_path.extension().unwrap_or_default() == "yml" {
        serde_yaml::from_str(&config_str)?
    } else if config_path.extension().unwrap_or_default() == "json" {
        serde_json::from_str(&config_str)?
    } else {
        return Err(anyhow!("不支持的配置文件格式"));
    };
    
    // 更新全局配置
    let mut global_config = CONFIG.write().await;
    *global_config = config;
    
    info!("配置加载成功: {}", config_path.display());
    
    // 设置文件监听器，用于监控配置文件变化
    setup_config_watcher(config_path)?;
    
    Ok(())
}

/// 设置配置文件监听器
fn setup_config_watcher(config_path: &Path) -> Result<()> {
    // 保存路径相关信息，避免移动后的借用问题
    let path_display = format!("{}", config_path.display());
    let config_path = config_path.to_path_buf();
    
    // 由于需要在外部调用watch，先保留一个父目录路径
    let parent_path = config_path.parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if event.kind.is_modify() || event.kind.is_create() {
                        info!("配置文件已更新，重新加载配置: {:?}", event);
                        
                        // 异步重新加载配置
                        let config_path_clone = config_path.clone();
                        tokio::spawn(async move {
                            match std::fs::read_to_string(&config_path_clone) {
                                Ok(config_str) => {
                                    // 使用anyhow::Result来统一错误类型
                                    let config_result: anyhow::Result<GatewayConfig> = 
                                        if config_path_clone.extension().unwrap_or_default() == "yaml" 
                                           || config_path_clone.extension().unwrap_or_default() == "yml" {
                                            serde_yaml::from_str(&config_str).map_err(|e| anyhow!(e))
                                        } else if config_path_clone.extension().unwrap_or_default() == "json" {
                                            serde_json::from_str(&config_str).map_err(|e| anyhow!(e))
                                        } else {
                                            Err(anyhow!("不支持的配置文件格式"))
                                        };
                                    
                                    match config_result {
                                        Ok(new_config) => {
                                            let mut global_config = CONFIG.write().await;
                                            *global_config = new_config;
                                            info!("热更新配置成功");
                                        },
                                        Err(e) => {
                                            error!("解析配置文件失败: {}", e);
                                        }
                                    }
                                },
                                Err(e) => {
                                    error!("读取配置文件失败: {}", e);
                                }
                            }
                        });
                    }
                },
                Err(e) => error!("监听配置文件变化错误: {}", e),
            }
        },
        Config::default(),
    )?;
    
    // 开始监听配置文件
    watcher.watch(&parent_path, RecursiveMode::NonRecursive)?;
    
    // 将watcher保存到全局，保持其生命周期
    std::mem::forget(watcher);
    
    info!("已设置配置文件监听器: {}", path_display);
    
    Ok(())
} 