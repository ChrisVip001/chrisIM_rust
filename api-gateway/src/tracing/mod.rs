use crate::config::CONFIG;
use tracing::{info, Level};
use tracing_subscriber::fmt::Layer as FmtLayer;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter};

/// 初始化链路追踪
pub async fn init_tracer() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 读取配置
    let config = CONFIG.read().await;

    // 设置日志级别为DEBUG
    let env_filter = EnvFilter::from_default_env()
        .add_directive(Level::DEBUG.into())
        .add_directive("hyper=info".parse().unwrap())  // 限制hyper日志
        .add_directive("tower=info".parse().unwrap()); // 限制tower日志

    // 如果未启用OpenTelemetry，只设置标准日志
    if !config.tracing.enable_opentelemetry {
        let fmt_layer = FmtLayer::new();

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();

        info!("已初始化日志系统，未启用OpenTelemetry链路追踪");
        return Ok(());
    }

    // 如果启用OpenTelemetry，我们在这里简化实现
    // 由于版本兼容性问题，我们暂时只使用标准日志
    info!("由于OpenTelemetry版本兼容性问题，暂时只使用标准日志");

    // 使用标准格式输出
    let fmt_layer = FmtLayer::new();

    // 初始化订阅者
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    info!("已初始化日志系统");

    Ok(())
}
