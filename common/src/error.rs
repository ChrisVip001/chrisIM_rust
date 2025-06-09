use aws_sdk_s3::error::SdkError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::StdError;
use serde_json::json;
use thiserror::Error;
use std::fmt;

#[derive(Debug, Error)]
pub enum Error {
    #[error("内部服务错误: {0}")]
    Internal(String),

    #[error("认证失败: {0}")]
    Authentication(String),

    #[error("授权失败: {0}")]
    Authorization(String),

    #[error("未授权访问")]
    Unauthorized,

    #[error("Token已过期")]
    TokenExpired,

    #[error("Token无效")]
    InvalidToken,

    #[error("签发者无效")]
    InvalidIssuer,

    #[error("没有足够的权限")]
    InsufficientPermissions,

    #[error("资源不存在: {0}")]
    NotFound(String),

    #[error("请求无效: {0}")]
    BadRequest(String),

    #[error("数据库错误: {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("mongodb错误: {0}")]
    MongoDB(#[from] mongodb::error::Error),

    #[error("Redis错误: {0}")]
    Redis(String),

    #[error("IO错误: {0}")]
    IO(#[from] std::io::Error),

    #[error("JSON错误: {0}")]
    Json(#[from] serde_json::Error),
    
    #[error("BinCodeDecode错误: {0}")]
    BinCodeDecode(#[from] bincode::error::DecodeError),
    
    #[error("BinCodeEncode错误: {0}")]
    BinCodeEncode(#[from] bincode::error::EncodeError),
    
    #[error("JWT错误: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("gRPC传输错误: {0}")]
    Tonic(#[from] tonic::transport::Error),

    #[error("gRPC状态错误: {0}")]
    TonicStatus(#[from] tonic::Status),

    #[error("对象存储服务错误")]
    OSSError,
    
    #[error("短信服务错误: {0}")]
    Sms(String),

    #[error("广播错误: {0}")]
    BroadCastError(String),
}

/// 增强的错误上下文，包含详细的错误位置信息
#[derive(Debug)]
pub struct ErrorContext {
    pub error: Error,
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
    pub target: &'static str,
    pub thread: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub additional_context: Option<String>,
}

impl ErrorContext {
    /// 创建错误上下文
    pub fn new(
        error: Error,
        file: &'static str,
        line: u32,
        column: u32,
        target: &'static str,
    ) -> Self {
        Self {
            error,
            file,
            line,
            column,
            target,
            thread: std::thread::current()
                .name()
                .unwrap_or("未知线程")
                .to_string(),
            timestamp: chrono::Local::now(),
            additional_context: None,
        }
    }

    /// 添加额外的上下文信息
    pub fn with_context<S: Into<String>>(mut self, context: S) -> Self {
        self.additional_context = Some(context.into());
        self
    }

    /// 记录错误到日志系统
    pub fn log_error(&self) {
        crate::logging::error_with_location!(
            error = %self.error,
            file = self.file,
            line = self.line,
            column = self.column,
            target = self.target,
            thread = %self.thread,
            timestamp = %self.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            context = ?self.additional_context,
            "发生错误"
        );
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "错误: {} | 位置: {}:{}:{} | 模块: {} | 线程: {} | 时间: {}",
            self.error,
            self.file,
            self.line,
            self.column,
            self.target,
            self.thread,
            self.timestamp.format("%Y-%m-%d %H:%M:%S%.3f")
        )?;
        
        if let Some(context) = &self.additional_context {
            write!(f, " | 上下文: {}", context)?;
        }
        
        Ok(())
    }
}

impl From<String> for Error {
    fn from(err: String) -> Self {
        Error::Internal(err)
    }
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Internal(err.to_string())
    }
}

// Redis错误转换实现
impl From<redis::RedisError> for Error {
    fn from(err: redis::RedisError) -> Self {
        Error::Redis(format!("Redis错误: {}", err))
    }
}

// 添加UUID解析错误的From实现
impl From<uuid::Error> for Error {
    fn from(err: uuid::Error) -> Self {
        Error::BadRequest(format!("UUID解析错误: {}", err))
    }
}

// 从Error转换为tonic::Status，用于gRPC响应
impl From<Error> for tonic::Status {
    fn from(error: Error) -> Self {
        match error {
            Error::NotFound(msg) => tonic::Status::not_found(msg),
            Error::Authentication(msg) => tonic::Status::unauthenticated(msg),
            Error::Authorization(msg) => tonic::Status::permission_denied(msg),
            Error::BadRequest(msg) => tonic::Status::invalid_argument(msg),
            Error::Sms(msg) => tonic::Status::unavailable(msg),
            _ => tonic::Status::internal(error.to_string()),
        }
    }
}

impl<E> From<SdkError<E>> for Error
where
    E: StdError + 'static,
{
    fn from(_err: SdkError<E>) -> Self {
        Error::OSSError
    }
}

// 从Error转换为axum::http::StatusCode，用于HTTP响应
impl From<Error> for axum::http::StatusCode {
    fn from(error: Error) -> Self {
        use axum::http::StatusCode;
        match error {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Authentication(_) => StatusCode::UNAUTHORIZED,
            Error::Authorization(_) => StatusCode::FORBIDDEN,
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::Sms(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, "未授权访问".to_string()),
            Error::TokenExpired => (StatusCode::UNAUTHORIZED, "Token已过期".to_string()),
            Error::InvalidToken => (StatusCode::UNAUTHORIZED, "Token无效".to_string()),
            Error::InvalidIssuer => (StatusCode::UNAUTHORIZED, "签发者无效".to_string()),
            Error::InsufficientPermissions => (StatusCode::FORBIDDEN, "没有足够的权限".to_string()),
            Error::Authentication(msg) => (
                StatusCode::UNAUTHORIZED,
                format!("认证失败: {}", msg),
            ),
            Error::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{}", msg),
            ),
            Error::Sms(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("短信服务错误: {}", msg),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "服务器内部错误".to_string(),
            ),
        };

        let json = Json(json!({
            "code": status.as_u16(),
            "message": message,
            "success": false
        }));

        (status, json).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// 增强的错误创建宏，自动捕获文件名、行号等位置信息
#[macro_export]
macro_rules! create_error {
    ($error_type:expr) => {
        $crate::error::ErrorContext::new(
            $error_type,
            file!(),
            line!(),
            column!(),
            module_path!(),
        )
    };
    ($error_type:expr, $context:expr) => {
        $crate::error::ErrorContext::new(
            $error_type,
            file!(),
            line!(),
            column!(),
            module_path!(),
        ).with_context($context)
    };
}

/// 错误记录并返回宏，用于简化错误处理流程
#[macro_export]
macro_rules! log_and_return_error {
    ($error:expr) => {{
        let error_ctx = create_error!($error);
        error_ctx.log_error();
        Err(error_ctx.error)
    }};
    ($error:expr, $context:expr) => {{
        let error_ctx = create_error!($error, $context);
        error_ctx.log_error();
        Err(error_ctx.error)
    }};
}

/// 错误映射宏，用于将一种错误转换为另一种错误并记录
#[macro_export]
macro_rules! map_error {
    ($result:expr, $error_mapper:expr) => {
        $result.map_err(|e| {
            let mapped_error = $error_mapper(e);
            let error_ctx = create_error!(mapped_error);
            error_ctx.log_error();
            error_ctx.error
        })
    };
    ($result:expr, $error_mapper:expr, $context:expr) => {
        $result.map_err(|e| {
            let mapped_error = $error_mapper(e);
            let error_ctx = create_error!(mapped_error, $context);
            error_ctx.log_error();
            error_ctx.error
        })
    };
}

// 重新导出宏
pub use create_error;
pub use log_and_return_error;
pub use map_error;
