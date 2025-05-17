use crate::config::CONFIG;
use tracing::info;
use anyhow::Result;

/// 初始化链路追踪
pub async fn init_tracer() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 读取配置
    let config = CONFIG.read().await;

    // 如果未启用OpenTelemetry，只设置标准日志
    if !config.tracing.enable_opentelemetry {
        // 使用common模块的日志初始化功能，同时启用sqlx日志
        common::logging::init_with_sqlx_level("debug")?;
        
        info!("已初始化日志系统，未启用OpenTelemetry链路追踪");
        return Ok(());
    }

    // 创建临时AppConfig结构体
    let telemetry_config = common::config::TelemetryConfig {
        enabled: config.tracing.enable_opentelemetry,
        endpoint: config.tracing.jaeger_endpoint.clone().unwrap_or_else(|| "http://localhost:4317".to_string()),
        sampling_ratio: config.tracing.sampling_ratio,
        propagation: "tracecontext".to_string(),
    };

    // 创建一个临时AppConfig实例
    let mut temp_config = common::config::AppConfig::new().map_err(|e| {
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;
    
    // 替换链路追踪配置
    temp_config.telemetry = telemetry_config;
    
    // 设置日志格式为JSON，便于日志聚合
    temp_config.log.format = Some("json".to_string());
    
    // 使用新实现的链路追踪功能
    common::logging::init_telemetry(&temp_config, "api-gateway")?;
    
    info!("已初始化链路追踪功能，追踪数据将发送到: {}", 
          config.tracing.jaeger_endpoint.as_deref().unwrap_or("http://localhost:4317"));

    Ok(())
}
