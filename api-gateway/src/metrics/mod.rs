use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use futures::future::BoxFuture;
use metrics::{counter, histogram, describe_counter, describe_histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use once_cell::sync::Lazy;
use std::time::Instant;
use tower::{Layer, Service};
use tracing::info;

// 全局 Prometheus 处理器
static PROMETHEUS_HANDLE: Lazy<PrometheusHandle> = Lazy::new(|| {
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9090))
        .install_recorder()
        .expect("Failed to install Prometheus recorder")
});

/// 初始化指标系统
pub fn init_metrics() {
    // 强制初始化 Prometheus 处理器
    Lazy::force(&PROMETHEUS_HANDLE);

    // 描述指标
    describe_counter!("gateway_requests_total", "Total number of requests processed by the gateway");
    describe_counter!("gateway_responses_total", "Total number of responses sent by the gateway");
    describe_counter!("gateway_errors_total", "Total number of errors encountered by the gateway");
    describe_histogram!("gateway_request_duration_seconds", "Request processing duration in seconds");

    info!("指标系统已初始化");
}

/// 指标请求处理函数
pub async fn get_metrics_handler() -> impl IntoResponse {
    let metrics = PROMETHEUS_HANDLE.render();
    (StatusCode::OK, metrics)
}

/// 指标中间件层
#[derive(Clone)]
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
        let path = req.uri().path().to_string();
        let method = req.method().to_string();
        let service = extract_service_name(&path);

        // 记录请求
        counter!("gateway_requests_total", 
            "method" => method.clone(),
            "path" => path.clone(),
            "service" => service.clone()
        );

        let start = Instant::now();
        let mut svc = self.inner.clone();

        Box::pin(async move {
            let result = svc.call(req).await;
            let duration = start.elapsed().as_secs_f64();

            // 记录请求处理时间
            histogram!("gateway_request_duration_seconds").record(duration);

            match &result {
                Ok(response) => {
                    let status = response.status().as_u16().to_string();

                    // 记录响应
                    counter!("gateway_responses_total",
                        "method" => method.clone(),
                        "path" => path.clone(),
                        "service" => service.clone(),
                        "status" => status.clone()
                    );

                    // 记录错误（4xx, 5xx）
                    if response.status().is_client_error() || response.status().is_server_error() {
                        counter!("gateway_errors_total",
                            "method" => method,
                            "path" => path,
                            "service" => service,
                            "status" => status
                        );
                    }
                }
                Err(_) => {
                    // 记录服务错误
                    counter!("gateway_errors_total",
                        "method" => method,
                        "path" => path,
                        "service" => service,
                        "status" => "service_error"
                    );
                }
            }

            result
        })
    }
}

/// 从路径中提取服务名称
fn extract_service_name(path: &str) -> String {
    if path.starts_with("/api/user") {
        "user".to_string()
    } else if path.starts_with("/api/friend") {
        "friend".to_string()
    } else if path.starts_with("/api/group") {
        "group".to_string()
    } else if path.starts_with("/metrics") {
        "metrics".to_string()
    } else {
        "unknown".to_string()
    }
}
