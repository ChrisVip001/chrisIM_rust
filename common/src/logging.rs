use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::{fmt, EnvFilter,fmt::time::FormatTime,fmt::format::Writer};
use std::env;
use std::path::Path;
use std::fs::{self, File};
use std::io::Write;
use chrono::Local;
// 新增导入，用于链路追踪
#[cfg(feature = "telemetry")]
use opentelemetry::global;
#[cfg(feature = "telemetry")]
use opentelemetry::sdk::propagation::TraceContextPropagator;
#[cfg(feature = "telemetry")]
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;


// 日志输出时间格式
struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

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

// 日志输出位置类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogOutput {
    // 控制台输出
    Console,
    // 文件输出
    File,
}

impl LogOutput {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "file" => LogOutput::File,
            _ => LogOutput::Console,
        }
    }
}

/// 设置自定义panic处理器，在程序panic时记录详细的错误信息
/// 包括panic消息、文件位置、调用堆栈等关键信息
pub fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let location = if let Some(location) = panic_info.location() {
            format!("文件: {}, 行号: {}, 列号: {}", 
                location.file(), 
                location.line(), 
                location.column()
            )
        } else {
            "位置未知".to_string()
        };
        
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            format!("Panic消息: {}", s)
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            format!("Panic消息: {}", s)
        } else {
            "Panic消息: 未知错误类型".to_string()
        };
        
        // 使用tracing记录panic信息
        tracing::error!(
            location = location,
            message = message,
            thread = ?std::thread::current().name().unwrap_or("未知线程"),
            "程序发生严重错误(Panic)，即将退出"
        );
        
        // 同时输出到stderr，确保在日志系统失效时也能看到错误
        eprintln!("💥 程序发生严重错误！");
        eprintln!("📍 {}", location);
        eprintln!("💬 {}", message);
        eprintln!("🧵 线程: {}", std::thread::current().name().unwrap_or("未知线程"));
        eprintln!("💡 请检查上述文件和行号以定位问题");
    }));
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
    setup_panic_hook();
    init_with_sqlx_level("debug")
}

/// 使用指定的SQLx日志级别初始化日志系统
pub fn init_with_sqlx_level(sqlx_level: &str) -> Result<()> {
    setup_panic_hook();
    
    // 创建过滤器字符串，直接设置sqlx级别
    let filter_string = format!("info,sqlx={}", sqlx_level);
    
    // 使用字符串创建过滤器
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(filter_string));
    
    // 初始化日志订阅器，启用更详细的代码位置信息
    fmt()
        .with_env_filter(env_filter)
        .with_timer(LocalTimer)
        .with_ansi(true) // 支持ANSI颜色
        .with_thread_names(true) // 显示线程名称
        .with_target(true) // 显示目标模块路径
        .with_file(true) // 显示文件名
        .with_line_number(true) // 显示行号
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
    setup_panic_hook();
    
    // 构建过滤器字符串
    let mut filter_parts = vec!["info".to_string()];  // 默认全局级别
    
    for (target, level) in directives {
        filter_parts.push(format!("{}={}", target, level));
    }
    
    let filter_string = filter_parts.join(",");
    
    // 使用字符串创建过滤器
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(filter_string));
    
    // 初始化日志订阅器，启用详细的位置信息
    fmt()
        .with_env_filter(env_filter)
        .with_timer(LocalTimer)
        .with_ansi(true)
        .with_thread_names(true)
        .with_target(true) // 显示目标模块路径
        .with_file(true) // 显示文件名
        .with_line_number(true) // 显示行号
        .init();
    
    info!("日志系统初始化成功，使用自定义过滤器");
    for (target, level) in directives {
        info!("- {}={}", target, level);
    }
    
    Ok(())
}

/// 确保日志目录存在
fn ensure_log_dir(dir_path: &str) -> Result<()> {
    let path = Path::new(dir_path);
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

/// 获取日志文件路径
fn get_log_file_path(service_name: &str) -> String {
    let date = Local::now().format("%Y-%m-%d").to_string();
    
    // 首先尝试从环境变量获取日志目录
    let log_dir = std::env::var("LOG_DIR").unwrap_or_else(|_| {
        // 检查是否在生产环境
        if let Ok(env) = std::env::var("ENVIRONMENT") {
            if env == "production" || env == "staging" {
                // 生产环境使用绝对路径
                "/var/log/rustim".to_string()
            } else {
                // 开发环境使用相对路径
                "logs".to_string()
            }
        } else {
            // 默认使用相对路径
            "logs".to_string()
        }
    });
    
    // 确保日志目录存在
    if let Err(e) = ensure_log_dir(&log_dir) {
        eprintln!("创建日志目录失败: {}", e);
    }
    
    // 文件格式：logs/服务名-日期.log
    format!("{}/{}-{}.log", log_dir, service_name, date)
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
///     logging::init_from_config(&config,"service")?;
///     tracing::info!("日志系统从配置初始化成功");
///     Ok(())
/// }
/// ```
pub fn init_from_config(config: &crate::config::AppConfig,service_name: &str) -> Result<()> {
    setup_panic_hook();
    
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
    
    // 确定日志输出位置
    let log_output = LogOutput::from_str(&config.log.output);

    
    // 根据配置的输出格式和位置选择日志输出方式
    match (log_format, log_output) {
        (LogFormat::Plain, LogOutput::Console) => {
            fmt()
                .with_env_filter(env_filter)
                .with_timer(LocalTimer)
                .with_ansi(true)
                .with_thread_names(true)
                .with_target(true) // 显示目标模块路径
                .with_file(true) // 显示文件名  
                .with_line_number(true) // 显示行号
                .init();
        }
        (LogFormat::Json, LogOutput::Console) => {
            fmt()
                .with_env_filter(env_filter)
                .with_timer(LocalTimer)
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_thread_names(true)
                .with_target(true) // 显示目标模块路径
                .with_file(true) // 显示文件名
                .with_line_number(true) // 显示行号
                .init();
        }
        (LogFormat::Plain, LogOutput::File) => {
            // 获取日志文件路径
            let log_file_path = get_log_file_path(&service_name);
            
            // 尝试打开文件，如果失败则回退到控制台
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path) 
            {
                Ok(file) => {
                    // 创建控制台输出层
                    let console_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .with_ansi(true) // 控制台使用ANSI颜色
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true); // 显示行号
                    
                    // 创建文件输出层
                    let file_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .with_ansi(false) // 文件中不使用ANSI颜色
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true) // 显示行号
                        .with_writer(file);
                    
                    // 注册两个输出层
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(console_layer)
                        .with(file_layer)
                        .init();
                    
                    // 日志初始化信息同时显示在控制台
                    info!("日志系统初始化成功，同时输出到控制台和文件: {}", log_file_path);
                }
                Err(e) => {
                    // 无法打开日志文件，回退到控制台
                    eprintln!("无法打开日志文件 {}: {}，回退到控制台输出", log_file_path, e);
                    fmt()
                        .with_env_filter(env_filter)
                        .with_timer(LocalTimer)
                        .with_ansi(true)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true) // 显示行号
                        .init();
                }
            }
        }
        (LogFormat::Json, LogOutput::File) => {
            // 获取日志文件路径
            let log_file_path = get_log_file_path(&service_name);
            
            // 尝试打开文件，如果失败则回退到控制台
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path) 
            {
                Ok(file) => {
                    // 创建控制台输出层 (使用JSON格式)
                    let console_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true); // 显示行号
                    
                    // 创建JSON文件输出层
                    let json_file_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true) // 显示行号
                        .with_writer(file);
                    
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(console_layer)
                        .with(json_file_layer)
                        .init();
                    
                    // 日志初始化信息同时显示在控制台
                    info!("日志系统初始化成功，同时输出到控制台(JSON格式)和文件(JSON格式): {}", log_file_path);
                }
                Err(e) => {
                    // 无法打开日志文件，回退到控制台
                    eprintln!("无法打开日志文件 {}: {}，回退到控制台输出", log_file_path, e);
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
                        .init();
                }
            }
        }
    }

    info!("日志系统从配置初始化成功，全局级别: {}", config.log.level);
    info!("SQLx日志级别: {}", config.log.sqlx_level());
    info!("日志格式: {:?}, 输出位置: {:?}", log_format, log_output);
    info!("获取配置路径: {}", std::env::var("CONFIG_PATH").unwrap_or_else(|_| "本地".to_string()));

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
    setup_panic_hook();
    
    // 首先检查环境变量 RUST_LOG
    if let Ok(_env_filter) = std::env::var("RUST_LOG") {
        return init_with_custom_filter(&[("sqlx", "debug")]);
    }
    
    // 如果没有环境变量，尝试从配置文件加载
    match crate::config::AppConfig::new() {
        Ok(config) => init_from_config(&config,"service_name"),
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
            // 获取日志文件路径
            let log_file_path = get_log_file_path(service_name);
            
            // 尝试打开文件，如果失败则回退到控制台
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path) 
            {
                Ok(file) => {
                    // 创建控制台输出层
                    let console_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .with_ansi(true)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true); // 显示行号
                    
                    // 创建文件输出层
                    let file_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .with_ansi(false)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true) // 显示行号
                        .with_writer(file);
                    
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(console_layer)
                        .with(file_layer)
                        .with(telemetry)
                        .init();
                    
                    // 日志初始化信息同时显示在控制台
                    info!("日志系统初始化成功（带分布式链路追踪），同时输出到控制台和文件: {}", log_file_path);
                    info!("链路追踪数据发送至: {}", jaeger_endpoint);
                }
                Err(e) => {
                    // 无法打开日志文件，回退到控制台
                    eprintln!("无法打开日志文件 {}: {}，回退到控制台输出", log_file_path, e);
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
            }
        }
        (LogFormat::Json, LogOutput::File) => {
            // 获取日志文件路径
            let log_file_path = get_log_file_path(service_name);
            
            // 尝试打开文件，如果失败则回退到控制台
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path) 
            {
                Ok(file) => {
                    // 创建控制台输出层 (使用JSON格式)
                    let console_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true); // 显示行号
                    
                    // 创建JSON文件输出层
                    let json_file_layer = fmt::layer()
                        .with_timer(LocalTimer)
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_thread_names(true)
                        .with_target(true) // 显示目标模块路径
                        .with_file(true) // 显示文件名
                        .with_line_number(true) // 显示行号
                        .with_writer(file);
                    
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(console_layer)
                        .with(json_file_layer)
                        .with(telemetry)
                        .init();
                    
                    // 日志初始化信息同时显示在控制台
                    info!("日志系统初始化成功（带分布式链路追踪），同时输出到控制台(JSON格式)和文件(JSON格式): {}", log_file_path);
                    info!("链路追踪数据发送至: {}", jaeger_endpoint);
                }
                Err(e) => {
                    // 无法打开日志文件，回退到控制台
                    eprintln!("无法打开日志文件 {}: {}，回退到控制台输出", log_file_path, e);
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

/// 错误追踪宏，用于在函数中记录详细的错误信息
/// 包括文件名、行号、函数名等位置信息
#[macro_export]
macro_rules! error_with_location {
    ($($arg:tt)*) => {
        tracing::error!(
            target: module_path!(),
            file = file!(),
            line = line!(),
            column = column!(),
            $($arg)*
        )
    };
}

/// 警告追踪宏，用于在函数中记录详细的警告信息
#[macro_export]
macro_rules! warn_with_location {
    ($($arg:tt)*) => {
        tracing::warn!(
            target: module_path!(),
            file = file!(),
            line = line!(),
            column = column!(),
            $($arg)*
        )
    };
}

/// 信息追踪宏，用于记录带位置信息的信息日志
#[macro_export]
macro_rules! info_with_location {
    ($($arg:tt)*) => {
        tracing::info!(
            target: module_path!(),
            file = file!(),
            line = line!(),
            column = column!(),
            $($arg)*
        )
    };
}

/// 调试追踪宏，用于记录带位置信息的调试日志
#[macro_export]
macro_rules! debug_with_location {
    ($($arg:tt)*) => {
        tracing::debug!(
            target: module_path!(),
            file = file!(),
            line = line!(),
            column = column!(),
            $($arg)*
        )
    };
}

/// 函数执行计时宏，自动记录函数执行时间
#[macro_export]
macro_rules! timed_function {
    ($func_name:expr, $block:block) => {{
        let start = std::time::Instant::now();
        tracing::debug!(
            target: module_path!(),
            file = file!(),
            line = line!(),
            function = $func_name,
            "开始执行函数"
        );
        
        let result = $block;
        
        let duration = start.elapsed();
        tracing::debug!(
            target: module_path!(),
            file = file!(),
            line = line!(),
            function = $func_name,
            duration_ms = duration.as_millis(),
            "函数执行完成"
        );
        
        result
    }};
}

// 重新导出宏，以便其他模块可以使用
pub use error_with_location;
pub use warn_with_location;  
pub use info_with_location;
pub use debug_with_location;
pub use timed_function; 