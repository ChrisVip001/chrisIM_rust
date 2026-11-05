//! Prometheus 指标
//!
//! `path` 标签归一化成 `/api/<服务>/<方法>`，动态段替换成 `{id}`，
//! 避免按用户 ID 生成海量时间序列。
//!
//! 只装 recorder（`build_recorder`），不额外起监听端口，
//! 指标由业务的 `metrics_endpoint` 路由渲染。

use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use futures::future::BoxFuture;
use metrics::{counter, describe_counter, describe_histogram, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;
use std::time::Instant;
use tower::{Layer, Service};
use tracing::{info, warn};

use crate::api_utils::{path_under, upstream_service};

/// 全局 Prometheus 处理器；`None` 表示 recorder 装不上，指标功能整体降级
static PROMETHEUS_HANDLE: OnceCell<Option<PrometheusHandle>> = OnceCell::new();

/// 初始化指标系统（幂等，装不上只告警不 panic）
pub fn init_metrics() {
    let handle = PROMETHEUS_HANDLE.get_or_init(|| {
        // 用 build_recorder：不涉及 HTTP 监听，指标由本进程的 `/metrics` 路由渲染
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        match metrics::set_global_recorder(recorder) {
            Ok(()) => Some(handle),
            Err(e) => {
                warn!("Prometheus recorder 安装失败，指标功能关闭: {}", e);
                None
            }
        }
    });

    if handle.is_none() {
        return;
    }

    describe_counter!("gateway_requests_total", "进入网关的请求数（method/path/service）");
    describe_counter!("gateway_responses_total", "网关产出的响应数（含 status）");
    describe_counter!("gateway_errors_total", "4xx/5xx 响应数");
    describe_histogram!("gateway_request_duration_seconds", "请求耗时（service）");

    // 以 0 值登记一次，空载抓取也不是空响应
    counter!("gateway_requests_total", "method" => "GET", "path" => "/health", "service" => "gateway")
        .increment(0);

    info!("指标系统已初始化");
}

/// 指标请求处理函数
pub async fn get_metrics_handler() -> impl IntoResponse {
    match PROMETHEUS_HANDLE.get().and_then(|h| h.as_ref()) {
        Some(handle) => (StatusCode::OK, handle.render()),
        // 指标被降级时明确回 503，而不是一个空的 200（Prometheus 会把它当成「全是 0」）
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder unavailable".to_string(),
        ),
    }
}

/// 指标中间件层
#[derive(Clone, Default)]
pub struct MetricsLayer;

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsMiddleware { inner }
    }
}

/// 指标中间件
#[derive(Clone)]
pub struct MetricsMiddleware<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for MetricsMiddleware<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let raw_path = req.uri().path();
        let path = normalize_path(raw_path);
        let service = upstream_service(raw_path).unwrap_or("gateway");
        let method = req.method().as_str().to_string();

        counter!("gateway_requests_total",
            "method" => method.clone(),
            "path" => path.clone(),
            "service" => service
        )
        .increment(1);

        let start = Instant::now();
        let mut svc = self.inner.clone();

        Box::pin(async move {
            let result = svc.call(req).await;
            let duration = start.elapsed().as_secs_f64();

            histogram!("gateway_request_duration_seconds", "service" => service).record(duration);

            match &result {
                Ok(response) => {
                    let status = response.status().as_u16().to_string();
                    counter!("gateway_responses_total",
                        "method" => method.clone(),
                        "path" => path.clone(),
                        "service" => service,
                        "status" => status.clone()
                    )
                    .increment(1);
                    if response.status().is_client_error() || response.status().is_server_error() {
                        counter!("gateway_errors_total",
                            "method" => method,
                            "path" => path,
                            "service" => service,
                            "status" => status
                        )
                        .increment(1);
                    }
                }
                Err(_) => {
                    counter!("gateway_errors_total",
                        "method" => method,
                        "path" => path,
                        "service" => service,
                        "status" => "service_error"
                    )
                    .increment(1);
                }
            }

            result
        })
    }
}

/// 把 URI 归一化成低基数形态：`/api/users/123/profile?x=1` → `/api/users/{id}/profile`
///
/// 规则：`/api/<服务>/<段4>/...`。段 4 是「方法名」或是资源 ID ——
/// 纯数字、纯十六进制、或长度 ≥ 16 的一律视为 ID。
pub fn normalize_path(path: &str) -> String {
    if path == "/health" || path_under(path, "/metrics") {
        return path.to_string();
    }

    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let (Some("api"), Some(service), Some(action)) =
        (segments.next(), segments.next(), segments.next())
    else {
        // 非业务路径（含 404）统一折叠，避免任意 URI 造出无限序列
        return if path == "/" { "/".to_string() } else { "/other".to_string() };
    };

    let action = if looks_like_id(action) { "{id}" } else { action };
    let mut normalized = format!("/api/{service}/{action}");

    // 方法名之后如果还有段，说明带了资源 ID，整体折叠
    if segments.next().is_some() {
        normalized.push_str("/*");
    }
    normalized
}

fn looks_like_id(segment: &str) -> bool {
    !segment.is_empty()
        && (segment.chars().all(|c| c.is_ascii_digit())
            || (segment.len() >= 16 && segment.chars().all(|c| c.is_ascii_hexdigit())))
}

#[cfg(test)]
mod tests {
    use super::normalize_path;

    #[test]
    fn keeps_method_names() {
        assert_eq!(normalize_path("/api/users/getUserById"), "/api/users/getUserById");
        assert_eq!(normalize_path("/api/chat/send"), "/api/chat/send");
    }

    #[test]
    fn folds_ids_and_tails() {
        assert_eq!(normalize_path("/api/users/123"), "/api/users/{id}");
        assert_eq!(normalize_path("/api/users/1/profile"), "/api/users/{id}/*");
        // 24 位十六进制（ObjectId 风格）
        assert_eq!(
            normalize_path("/api/groups/507f1f77bcf86cd799439011"),
            "/api/groups/{id}"
        );
    }

    #[test]
    fn bounds_cardinality_for_unknown_paths() {
        assert_eq!(normalize_path("/whatever/deep/path"), "/other");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("/health"), "/health");
        assert_eq!(normalize_path("/metrics"), "/metrics");
    }
}
