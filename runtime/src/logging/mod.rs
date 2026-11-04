//! 日志模块
//!
//! 按职责拆分：
//! - 本文件：格式化层（时间/格式/输出）与 panic 钩子、定位日志宏
//! - [`init`]：日志订阅器初始化（控制台/滚动文件）、目录清理、环境变量过滤
//! - [`telemetry`]：OpenTelemetry 链路追踪接入
//!
//! 公共 API 由本文件统一再导出，调用方使用方式不变。
use anyhow::Result;
use chrono::Local;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

mod init;
mod telemetry;

pub use init::{
    init, init_auto, init_from_config, init_with_custom_filter, init_with_level,
    init_with_sqlx_level,
};
pub use telemetry::{init_telemetry, shutdown_telemetry};

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
/// use runtime::logging;
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
