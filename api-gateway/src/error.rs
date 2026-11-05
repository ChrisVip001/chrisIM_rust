//! 网关统一错误类型
//!
//! 对外只暴露一种形状：`{"code": <HTTP 状态码>, "message": "...", "success": false}`，
//! 且 **HTTP 状态码与 `code` 恒相等**。
//!
//! 对内：`detail` 只进日志，不进响应体。gRPC 的 `Status` 原文里带着服务名、端口、
//! 甚至 SQL 片段，回给客户端会泄漏内部拓扑与执行细节。

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tracing::{debug, error, warn};

/// 网关错误：状态码 + 可对外文案 + 仅日志可见的细节
#[derive(Debug)]
pub struct GatewayError {
    status: StatusCode,
    /// 可以安全回给客户端的文案（不含内部细节）
    public: String,
    /// 只写日志的原始错误
    detail: Option<String>,
}

impl GatewayError {
    fn new(status: StatusCode, public: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            status,
            public: public.into(),
            detail,
        }
    }

    /// 400：参数缺失、格式错误、畸形 JSON
    pub fn bad_request(public: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, public, None)
    }

    /// 指定状态码，文案直接对外（只用于确定不含内部细节的场景）
    pub fn with_status(status: StatusCode, public: impl Into<String>) -> Self {
        Self::new(status, public, None)
    }

    /// 401：未认证、凭据失效
    pub fn unauthorized(public: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, public, None)
    }

    /// 403：已认证但无权限
    pub fn forbidden(public: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, public, None)
    }

    /// 404：资源或接口不存在
    pub fn not_found(public: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, public, None)
    }

    /// 409：状态冲突
    pub fn conflict(public: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, public, None)
    }

    /// 429：限流
    pub fn too_many_requests(public: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, public, None)
    }

    /// 503：下游不可用（连不上、熔断、客户端未就绪）
    pub fn unavailable(detail: impl std::fmt::Display) -> Self {
        let detail = detail.to_string();
        warn!("下游服务不可用: {}", detail);
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "服务暂时不可用，请稍后重试", Some(detail))
    }

    /// 504：下游超时
    pub fn timeout(detail: impl std::fmt::Display) -> Self {
        let detail = detail.to_string();
        warn!("下游服务超时: {}", detail);
        Self::new(StatusCode::GATEWAY_TIMEOUT, "上游服务响应超时，请稍后重试", Some(detail))
    }

    /// 502：下游返回了错误
    pub fn upstream(detail: impl std::fmt::Display) -> Self {
        let detail = detail.to_string();
        warn!("下游服务返回错误: {}", detail);
        Self::new(StatusCode::BAD_GATEWAY, "上游服务处理失败", Some(detail))
    }

    /// 500：网关自身错误
    pub fn internal(detail: impl std::fmt::Display) -> Self {
        let detail = detail.to_string();
        error!("网关内部错误: {}", detail);
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "服务内部错误", Some(detail))
    }

    #[cfg(test)]
    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(d) => write!(f, "{} ({}): {}", self.status.as_u16(), self.public, d),
            None => write!(f, "{}: {}", self.status.as_u16(), self.public),
        }
    }
}

impl std::error::Error for GatewayError {}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        // 5xx 由构造函数打过日志；4xx 在 debug 级别留一行，便于排查又不会刷屏
        if self.status.is_server_error() {
            if let Some(detail) = &self.detail {
                error!(status = self.status.as_u16(), "{}: {}", self.public, detail);
            }
        } else {
            debug!(status = self.status.as_u16(), "{}", self.public);
        }

        (
            self.status,
            Json(json!({
                "code": self.status.as_u16(),
                "message": self.public,
                "success": false,
            })),
        )
            .into_response()
    }
}

/// tonic 的错误码 → HTTP 语义
///
/// 没有这张表时，下游说「参数错了」网关回 500，客户端无法区分「我传错了」
/// 和「服务挂了」，运维也无法用 5xx 比例做告警。
impl From<tonic::Status> for GatewayError {
    fn from(status: tonic::Status) -> Self {
        let msg = status.message().to_string();
        match status.code() {
            tonic::Code::InvalidArgument | tonic::Code::FailedPrecondition | tonic::Code::OutOfRange => {
                Self::bad_request(msg)
            }
            tonic::Code::Unauthenticated => Self::unauthorized(msg),
            tonic::Code::PermissionDenied => Self::forbidden(msg),
            tonic::Code::NotFound => Self::not_found(msg),
            tonic::Code::AlreadyExists | tonic::Code::Aborted => Self::conflict(msg),
            tonic::Code::ResourceExhausted => Self::too_many_requests(msg),
            tonic::Code::DeadlineExceeded => Self::timeout(format!("{status:?}")),
            tonic::Code::Unavailable => Self::unavailable(format!("{status:?}")),
            // 下游的 Internal / Unknown / DataLoss / Unimplemented 是它自己的问题，
            // 对客户端而言属于「上游失败」，不能让网关背 500 的锅
            _ => Self::upstream(format!("{status:?}")),
        }
    }
}

/// anyhow 错误一律折叠为 500：它没有可判定的语义，不该猜
impl From<anyhow::Error> for GatewayError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(format!("{err:#}"))
    }
}

/// 数字解析失败只可能来自客户端输入
impl From<std::num::ParseIntError> for GatewayError {
    fn from(err: std::num::ParseIntError) -> Self {
        Self::bad_request(format!("数字参数格式错误: {err}"))
    }
}

impl From<std::num::ParseFloatError> for GatewayError {
    fn from(err: std::num::ParseFloatError) -> Self {
        Self::bad_request(format!("数字参数格式错误: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(err: GatewayError) -> (StatusCode, serde_json::Value) {
        let status = err.status;
        let resp = err.into_response();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn envelope_code_matches_http_status() {
        for err in [
            GatewayError::bad_request("参数缺失"),
            GatewayError::unauthorized("令牌失效"),
            GatewayError::not_found("接口不存在"),
            GatewayError::internal("db down"),
        ] {
            let (status, body) = body_json(err).await;
            assert_eq!(body["code"], status.as_u16());
            assert_eq!(body["success"], false);
        }
    }

    #[tokio::test]
    async fn server_side_detail_never_leaks() {
        let (_, body) = body_json(GatewayError::internal(
            "postgres://root:pw@10.0.0.1:5432/boxim exploded",
        ))
        .await;
        let text = body.to_string();
        assert!(!text.contains("postgres"), "内部连接串泄漏到响应: {text}");
        assert!(!text.contains("10.0.0.1"), "内部地址泄漏到响应: {text}");
        assert_eq!(body["message"], "服务内部错误");
    }

    #[test]
    fn tonic_codes_map_to_sane_http_status() {
        let cases = [
            (tonic::Code::InvalidArgument, 400),
            (tonic::Code::Unauthenticated, 401),
            (tonic::Code::PermissionDenied, 403),
            (tonic::Code::NotFound, 404),
            (tonic::Code::AlreadyExists, 409),
            (tonic::Code::ResourceExhausted, 429),
            (tonic::Code::Unavailable, 503),
            (tonic::Code::DeadlineExceeded, 504),
            (tonic::Code::Internal, 502),
            (tonic::Code::Unknown, 502),
        ];
        for (code, want) in cases {
            let err: GatewayError = tonic::Status::new(code, "x").into();
            assert_eq!(err.status().as_u16(), want, "code={code:?}");
        }
    }
}
