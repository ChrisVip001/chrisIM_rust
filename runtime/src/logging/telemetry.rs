//! OpenTelemetry 链路追踪初始化与关闭
//!
//! 从 `logging.rs` 拆出；启用 `telemetry` feature 时接入 OTLP 导出器。

#[cfg(feature = "telemetry")]
use opentelemetry::global;
#[cfg(feature = "telemetry")]
use opentelemetry::sdk::propagation::TraceContextPropagator;
#[cfg(feature = "telemetry")]
use opentelemetry_otlp::WithExportConfig;
use anyhow::Result;
use tracing::info;
use tracing_appender::non_blocking;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::AppConfig;
use super::{LocalTimer, LogFormat, LogOutput};
use super::init::{cleanup_old_log_files, create_rolling_appender, ensure_log_dir, get_log_directory, init_from_config};
use super::setup_panic_hook;

#[cfg(feature = "telemetry")]
pub fn init_telemetry(config: &crate::config::AppConfig, service_name: &str) -> Result<()> {
    setup_panic_hook();
    
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
    
    // 确定日志输出位置
    let log_output = LogOutput::from_str(&config.log.output);


    
    // 创建OpenTelemetry层
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    
    // 根据配置的输出格式和位置选择日志输出方式并包含OpenTelemetry层
    match (log_format, log_output) {
        (LogFormat::Plain, LogOutput::Console) => {
            let fmt_layer = fmt::layer()
                .with_timer(LocalTimer)
                .with_ansi(true)
                .with_thread_names(true)
                .with_target(true) // 显示目标模块路径
                .with_file(true) // 显示文件名
                .with_line_number(true); // 显示行号
            
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .with(telemetry)
                .init();
        }
        (LogFormat::Json, LogOutput::Console) => {
            let json_layer = fmt::layer()
                .with_timer(LocalTimer)
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_thread_names(true)
                .with_target(true) // 显示目标模块路径
                .with_file(true) // 显示文件名
                .with_line_number(true); // 显示行号
            
            tracing_subscriber::registry()
                .with(env_filter)
                .with(json_layer)
                .with(telemetry)
                .init();
        }
        (LogFormat::Plain, LogOutput::File) => {
            // 检查是否有日志滚动配置
            if let Some(rolling_config) = config.log.rolling() {
                // 使用滚动日志
                match create_rolling_appender(rolling_config, service_name) {
                    Ok((non_blocking, _guard)) => {
                        // 创建控制台输出层
                        let console_layer = fmt::layer()
                            .with_timer(LocalTimer)
                            .with_ansi(true)
                            .with_thread_names(true)
                            .with_target(true)
                            .with_file(true)
                            .with_line_number(true);
                        
                        // 创建滚动文件输出层
                        let file_layer = fmt::layer()
                            .with_timer(LocalTimer)
                            .with_ansi(false)
                            .with_thread_names(true)
                            .with_target(true)
                            .with_file(true)
                            .with_line_number(true)
                            .with_writer(non_blocking);
                        
                        tracing_subscriber::registry()
                            .with(env_filter)
                            .with(console_layer)
                            .with(file_layer)
                            .with(telemetry)
                            .init();
                        
                        info!("日志系统初始化成功（带分布式链路追踪），使用滚动日志文件");
                        info!("链路追踪数据发送至: {}", jaeger_endpoint);
                        
                        // 将guard保存在全局变量中
                        std::mem::forget(_guard);
                    }
                    Err(e) => {
                        eprintln!("创建滚动日志失败: {}，回退到控制台输出", e);
                        let fmt_layer = fmt::layer()
                            .with_timer(LocalTimer)
                            .with_ansi(true)
                            .with_thread_names(true)
                            .with_target(true)
                            .with_file(true)
                            .with_line_number(true);
                        
                        tracing_subscriber::registry()
                            .with(env_filter)
                            .with(fmt_layer)
                            .with(telemetry)
                            .init();
                    }
                }
            } else {
                // 没有滚动配置，使用控制台输出
                let fmt_layer = fmt::layer()
                    .with_timer(LocalTimer)
                    .with_ansi(true)
                    .with_thread_names(true)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true);
                
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .with(telemetry)
                    .init();
                
                info!("日志系统初始化成功（带分布式链路追踪），使用控制台输出（未配置滚动日志）");
                info!("链路追踪数据发送至: {}", jaeger_endpoint);
            }
        }
        (LogFormat::Json, LogOutput::File) => {
            // 检查是否有日志滚动配置
            if let Some(rolling_config) = config.log.rolling() {
                // 使用滚动日志
                    match create_rolling_appender(rolling_config, service_name) {
                        Ok((non_blocking, _guard)) => {
                            // 创建控制台输出层 (使用JSON格式)
                            let console_layer = fmt::layer()
                                .with_timer(LocalTimer)
                                .json()
                                .with_current_span(true)
                                .with_span_list(true)
                                .with_thread_names(true)
                                .with_target(true)
                                .with_file(true)
                                .with_line_number(true);
                            
                            // 创建JSON滚动文件输出层
                            let json_file_layer = fmt::layer()
                                .with_timer(LocalTimer)
                                .json()
                                .with_current_span(true)
                                .with_span_list(true)
                                .with_thread_names(true)
                                .with_target(true)
                                .with_file(true)
                                .with_line_number(true)
                                .with_writer(non_blocking);
                            
                            tracing_subscriber::registry()
                                .with(env_filter)
                                .with(console_layer)
                                .with(json_file_layer)
                                .with(telemetry)
                                .init();
                            
                            info!("日志系统初始化成功（带分布式链路追踪），使用JSON格式的滚动日志文件");
                            info!("链路追踪数据发送至: {}", jaeger_endpoint);
                            
                            // 将guard保存在全局变量中
                            std::mem::forget(_guard);
                        }
                        Err(e) => {
                            eprintln!("创建滚动日志失败: {}，回退到控制台输出", e);
                            let json_layer = fmt::layer()
                                .with_timer(LocalTimer)
                                .json()
                                .with_current_span(true)
                                .with_span_list(true)
                                .with_thread_names(true)
                                .with_target(true)
                                .with_file(true)
                                .with_line_number(true);
                            
                            tracing_subscriber::registry()
                                .with(env_filter)
                                .with(json_layer)
                                .with(telemetry)
                                .init();
                        }
                    }

            } else {
                // 没有滚动配置，使用控制台输出
                let json_layer = fmt::layer()
                    .with_timer(LocalTimer)
                    .json()
                    .with_current_span(true)
                    .with_span_list(true)
                    .with_thread_names(true)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true);
                
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(json_layer)
                    .with(telemetry)
                    .init();
                
                info!("日志系统初始化成功（带分布式链路追踪），使用JSON格式控制台输出（未配置滚动日志）");
                info!("链路追踪数据发送至: {}", jaeger_endpoint);
            }
        }
    }
    
    // 无需特殊条件判断，日志信息已经在各个分支中处理
    
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
    init_from_config(_config,service_name)
}

#[cfg(not(feature = "telemetry"))]
pub fn shutdown_telemetry() {
    // 没有实际操作
}
