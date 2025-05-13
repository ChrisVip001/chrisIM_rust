use std::time::Instant;
use tower::Layer;
use tower::Service;
use futures::future::BoxFuture;
use metrics::{counter, histogram};
use prometheus::{Registry, TextEncoder, Encoder};
use axum::{
    http::{Request, Response, StatusCode},
    body::Body,
    response::IntoResponse,
};
use tracing::info;
use std::sync::Arc;
use once_cell::sync::Lazy;

// 全局 Prometheus 注册表
static REGISTRY: Lazy<Arc<Registry>> = Lazy::new(|| {
    let registry = Registry::new();
    Arc::new(registry)
});

/// 获取全局Registry
pub fn get_registry() -> Arc<Registry> {
    REGISTRY.clone()
}

/// 初始化指标系统
pub fn init_metrics() {
    // 注册默认的收集器
    let _registry = get_registry();
    info!("Prometheus指标已初始化");
}

/// 指标请求处理函数
pub async fn get_metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let registry = get_registry();
    
    // 收集所有指标
    let metric_families = registry.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap_or_else(|e| {
        eprintln!("无法编码指标: {}", e);
    });
    
    let metrics_text = String::from_utf8(buffer).unwrap_or_else(|e| {
        eprintln!("无法将指标转换为UTF-8: {}", e);
        String::from("metrics encoding error")
    });
    
    (StatusCode::OK, metrics_text)
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

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // 获取请求路径
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        
        // 获取服务名称
        let service = extract_service_name(&path);
        
        // 增加请求计数
        counter!("gateway.requests.total", 
            "method" => method.to_string(), 
            "path" => path.clone(), 
            "service" => service.clone()
        );
        
        // 开始计时
        let start = Instant::now();
        
        // 克隆服务
        let mut svc = self.inner.clone();

        Box::pin(async move {
            let result = svc.call(req).await;
            
            // 计算请求处理时间
            let duration = start.elapsed();
            
            match &result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    
                    // 记录请求处理时间（以秒为单位）
                    let duration_secs = duration.as_secs_f64();
                    histogram!("gateway.request.duration").record(duration_secs);
                    
                    // 统计状态码
                    let path_clone = path.clone();
                    let service_clone = service.clone();
                    counter!("gateway.responses.total",
                        "method" => method.to_string(),
                        "path" => path_clone,
                        "service" => service_clone,
                        "status" => status.to_string()
                    );
                    
                    // 统计错误状态码
                    if status >= 400 {
                        counter!("gateway.errors.total",
                            "method" => method.to_string(),
                            "path" => path,
                            "service" => service,
                            "status" => status.to_string()
                        );
                    }
                }
                Err(_) => {
                    // 统计请求失败
                    counter!("gateway.errors.total",
                        "method" => method.to_string(),
                        "path" => path,
                        "service" => service,
                        "status" => "error"
                    );
                }
            }
            
            result
        })
    }
}

/// 从路径中提取服务名称
fn extract_service_name(path: &str) -> String {
    if path.starts_with("/api/auth") {
        "auth".to_string()
    } else if path.starts_with("/api/users") {
        "user".to_string()
    } else if path.starts_with("/api/friends") {
        "friend".to_string()
    } else if path.starts_with("/api/groups") {
        "group".to_string()
    } else if path.starts_with("/metrics") {
        "metrics".to_string()
    } else {
        "unknown".to_string()
    }
}

/// 创建指标中间件
pub fn metrics_middleware() -> impl tower::Layer<tower::util::BoxCloneService<axum::http::Request<Body>, axum::response::Response<Body>, tower::BoxError>> + Clone {
    // 创建MetricsLayer实例
    MetricsLayer
} 