use crate::config::CONFIG;
use tracing::{info, Level};
use tracing_subscriber::fmt::Layer as FmtLayer;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter};

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

    // 如果启用OpenTelemetry，我们在这里简化实现
    // 由于版本兼容性问题，我们暂时只使用标准日志
    info!("由于OpenTelemetry版本兼容性问题，暂时只使用标准日志");

    // 使用common模块的日志初始化功能，同时启用sqlx日志
    common::logging::init_with_sqlx_level("debug")?;

    info!("已初始化日志系统");

    Ok(())
}
