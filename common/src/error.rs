use aws_sdk_s3::error::SdkError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::StdError;
use serde_json::json;
use thiserror::Error;

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
            Error::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("内部服务错误: {}", msg),
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
            "error": status.as_u16(),
            "message": message,
        }));

        (status, json).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
