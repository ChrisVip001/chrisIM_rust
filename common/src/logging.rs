use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::{fmt, EnvFilter, prelude::*};
use std::env;

// 新增导入，用于链路追踪
#[cfg(feature = "telemetry")]
use opentelemetry::global;
#[cfg(feature = "telemetry")]
use tracing_opentelemetry::OpenTelemetryLayer;
#[cfg(feature = "telemetry")]
use opentelemetry::sdk::propagation::TraceContextPropagator;
#[cfg(feature = "telemetry")]
use opentelemetry_otlp::WithExportConfig;

// 日志输出格式类型
#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    // 普通文本格式
    Plain,
    // JSON格式，适合ELK等日志聚合系统
    Json,
}

impl LogFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => LogFormat::Json,
            _ => LogFormat::Plain,
        }
    }
}

/// 初始化日志系统
/// 
/// # 参数
/// * `sqlx_level` - SQLx库的日志级别，默认为"debug"
/// 
/// # 返回值
/// * `Result<()>` - 成功或失败的结果
/// 
/// # 示例
/// ```
/// use common::logging;
/// 
/// fn main() -> anyhow::Result<()> {
///     // 使用默认配置初始化日志
///     logging::init()?;
///     
///     // 或者指定SQLx日志级别
///     // logging::init_with_sqlx_level("info")?;
///     
///     tracing::info!("日志系统初始化成功");
///     Ok(())
/// }
/// ```
pub fn init() -> Result<()> {
    init_with_sqlx_level("debug")
}

/// 使用指定的SQLx日志级别初始化日志系统
pub fn init_with_sqlx_level(sqlx_level: &str) -> Result<()> {
    // 创建过滤器字符串，直接设置sqlx级别
    let filter_string = format!("info,sqlx={}", sqlx_level);
    
    // 使用字符串创建过滤器
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(filter_string));
    
    // 初始化日志订阅器
    fmt()
        .with_env_filter(env_filter)
        .with_ansi(true) // 支持ANSI颜色
        .with_thread_names(true) // 显示线程名称
        .init();
    
    info!("日志系统初始化成功，SQLx日志级别: {}", sqlx_level);
    Ok(())
}

// 为了保持向后兼容性，但实际上全局level参数已被忽略
pub fn init_with_level(_level: Level, sqlx_level: &str) -> Result<()> {
    init_with_sqlx_level(sqlx_level)
}

/// 自定义多组件日志级别，支持更复杂的日志配置
pub fn init_with_custom_filter(directives: &[(&str, &str)]) -> Result<()> {
    // 构建过滤器字符串
    let mut filter_parts = vec!["info".to_string()];  // 默认全局级别
    
    for (target, level) in directives {
        filter_parts.push(format!("{}={}", target, level));
    }
    
    let filter_string = filter_parts.join(",");
    
    // 使用字符串创建过滤器
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(filter_string));
    
    // 初始化日志订阅器
    fmt()
        .with_env_filter(env_filter)
        .with_ansi(true)
        .with_thread_names(true)
        .init();
    
    info!("日志系统初始化成功，使用自定义过滤器");
    for (target, level) in directives {
        info!("- {}={}", target, level);
    }
    
    Ok(())
}

/// 从配置初始化日志系统
/// 
/// # 参数
/// * `config` - 应用配置
/// 
/// # 返回值
/// * `Result<()>` - 成功或失败的结果
/// 
/// # 示例
/// ```
/// use common::config::AppConfig;
/// use common::logging;
/// 
/// fn main() -> anyhow::Result<()> {
///     let config = AppConfig::new()?;
///     logging::init_from_config(&config)?;
///     tracing::info!("日志系统从配置初始化成功");
///     Ok(())
/// }
/// ```
pub fn init_from_config(config: &crate::config::AppConfig) -> Result<()> {
    // 构建过滤器部分
    let mut filter_parts = vec![config.log.level.clone()];
    
    // 添加 sqlx 日志级别
    filter_parts.push(format!("sqlx={}", config.log.sqlx_level()));
    
    // 添加其他组件的日志级别
    if let Some(components) = &config.log.components {
        for (component, level) in components {
            filter_parts.push(format!("{}={}", component, level));
        }
    }
    
    // 构建过滤器字符串
    let filter_string = filter_parts.join(",");
    
    // 检查环境变量是否有覆盖设置
    let env_filter = if let Ok(env_filter) = std::env::var("RUST_LOG") {
        info!("使用环境变量 RUST_LOG={} 覆盖配置文件的日志级别", env_filter);
        EnvFilter::new(env_filter)
    } else {
        EnvFilter::new(filter_string)
    };
    
    // 检查是否有组件特定的环境变量覆盖
    let env_filter = check_env_component_overrides(env_filter);
    
    // 确定日志格式
    let log_format = if let Some(format) = &config.log.format {
        LogFormat::from_str(format)
    } else {
        LogFormat::Plain
    };
    
    // 根据配置的输出格式选择日志输出方式
    match log_format {
        LogFormat::Plain => {
            fmt()
                .with_env_filter(env_filter)
                .with_ansi(true)
                .with_thread_names(true)
                .init();
        }
        LogFormat::Json => {
            fmt()
                .with_env_filter(env_filter)
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_thread_names(true)
                .init();
        }
    }
    
    info!("日志系统从配置初始化成功，全局级别: {}", config.log.level);
    info!("SQLx日志级别: {}", config.log.sqlx_level());
    info!("日志格式: {:?}", log_format);
    
    if let Some(components) = &config.log.components {
        for (component, level) in components {
            info!("组件 {} 日志级别: {}", component, level);
        }
    }
    
    Ok(())
}

/// 检查环境变量中是否有组件特定的日志级别覆盖
fn check_env_component_overrides(mut env_filter: EnvFilter) -> EnvFilter {
    // 常见的组件名称
    let common_components = ["sqlx", "tower", "hyper", "axum", "tonic"];
    
    for component in &common_components {
        let env_var_name = format!("RUST_LOG_{}", component.to_uppercase());
        if let Ok(level) = env::var(&env_var_name) {
            info!("使用环境变量 {}={} 设置组件日志级别", env_var_name, level);
            // 尝试添加指令，如果解析失败则忽略
            if let Ok(directive) = format!("{}={}", component, level).parse() {
                env_filter = env_filter.add_directive(directive);
            }
        }
    }
    
    env_filter
}

/// 简化的日志初始化，同时支持环境变量和简单命令行参数
/// 
/// 该函数是一个便捷方法，只需要在程序入口处调用一次。
/// 它会检查环境变量和配置文件，并设置适当的日志级别。
/// 
/// # 返回值
/// * `Result<()>` - 成功或失败的结果
pub fn init_auto() -> Result<()> {
    // 首先检查环境变量 RUST_LOG
    if let Ok(_env_filter) = std::env::var("RUST_LOG") {
        return init_with_custom_filter(&[("sqlx", "debug")]);
    }
    
    // 如果没有环境变量，尝试从配置文件加载
    match crate::config::AppConfig::new() {
        Ok(config) => init_from_config(&config),
        Err(_) => {
            // 如果配置加载失败，使用默认设置
            info!("无法加载配置文件，使用默认日志设置");
            init_with_sqlx_level("debug")
        }
    }
}

/// 初始化支持OpenTelemetry分布式链路追踪的日志系统
/// 
/// # 参数
/// * `config` - 应用配置
/// * `service_name` - 服务名称，用于标识链路追踪中的服务
/// 
/// # 返回值
/// * `Result<()>` - 成功或失败的结果
#[cfg(feature = "telemetry")]
pub fn init_telemetry(config: &crate::config::AppConfig, service_name: &str) -> Result<()> {
    // 设置全局传播器为TraceContext
    global::set_text_map_propagator(TraceContextPropagator::new());
    
    // 构建Jaeger导出器，用于发送链路追踪数据
    let jaeger_endpoint = config.telemetry.endpoint.clone();
    let otlp_exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(jaeger_endpoint);
    
    // 配置OpenTelemetry
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(otlp_exporter)
        .with_trace_config(opentelemetry::sdk::trace::config().with_resource(
            opentelemetry::sdk::Resource::new(vec![
                opentelemetry::KeyValue::new("service.name", service_name.to_string()),
            ]),
        ))
        .install_batch(opentelemetry::runtime::Tokio)?;
    
    // 构建日志过滤器
    let mut filter_parts = vec![config.log.level.clone()];
    
    // 添加 sqlx 日志级别
    filter_parts.push(format!("sqlx={}", config.log.sqlx_level()));
    
    // 添加其他组件的日志级别
    if let Some(components) = &config.log.components {
        for (component, level) in components {
            filter_parts.push(format!("{}={}", component, level));
        }
    }
    
    // 构建过滤器字符串
    let filter_string = filter_parts.join(",");
    
    // 创建环境过滤器
    let env_filter = if let Ok(env_filter) = std::env::var("RUST_LOG") {
        info!("使用环境变量 RUST_LOG={} 覆盖配置文件的日志级别", env_filter);
        EnvFilter::new(env_filter)
    } else {
        EnvFilter::new(filter_string)
    };
    
    // 确定日志格式
    let log_format = if let Some(format) = &config.log.format {
        LogFormat::from_str(format)
    } else {
        LogFormat::Plain
    };
    
    // 创建OpenTelemetry层
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    
    // 根据配置的输出格式选择日志输出方式并包含OpenTelemetry层
    match log_format {
        LogFormat::Plain => {
            // 使用普通文本格式 + OpenTelemetry
            let fmt_layer = fmt::layer()
                .with_ansi(true)
                .with_thread_names(true);
            
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .with(telemetry)
                .init();
        }
        LogFormat::Json => {
            // 使用JSON格式 + OpenTelemetry
            let json_layer = fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_thread_names(true);
            
            tracing_subscriber::registry()
                .with(env_filter)
                .with(json_layer)
                .with(telemetry)
                .init();
        }
    }
    
    info!("日志系统初始化成功（带分布式链路追踪），服务名称: {}", service_name);
    info!("链路追踪数据发送至: {}", jaeger_endpoint);
    info!("日志格式: {:?}", log_format);
    
    Ok(())
}

/// 关闭OpenTelemetry，刷新剩余的跟踪数据
#[cfg(feature = "telemetry")]
pub fn shutdown_telemetry() {
    opentelemetry::global::shutdown_tracer_provider();
}

#[cfg(not(feature = "telemetry"))]
pub fn init_telemetry(_config: &crate::config::AppConfig, service_name: &str) -> Result<()> {
    info!("分布式链路追踪未启用 (缺少 'telemetry' 特性)，服务: {}", service_name);
    init_from_config(_config)
}

#[cfg(not(feature = "telemetry"))]
pub fn shutdown_telemetry() {
    // 没有实际操作
} 