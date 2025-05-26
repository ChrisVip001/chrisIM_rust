use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, Response},
};
use futures::future::BoxFuture;
use std::{
    net::SocketAddr,
    task::{Context, Poll},
    time::Instant,
};
use tower::{Layer, Service};
use tracing::{info, warn};

/// 请求日志中间件层
#[derive(Clone)]
pub struct RequestLoggerLayer;

impl<S> Layer<S> for RequestLoggerLayer {
    type Service = RequestLoggerMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestLoggerMiddleware { inner }
    }
}

/// 请求日志中间件
#[derive(Clone)]
pub struct RequestLoggerMiddleware<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for RequestLoggerMiddleware<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let client_ip = get_client_ip(&req);
        let user_agent = get_user_agent(&req);

        let start = Instant::now();
        let mut svc = self.inner.clone();

        Box::pin(async move {
            let result = svc.call(req).await;
            let duration = start.elapsed();

            match &result {
                Ok(response) => {
                    let status = response.status().as_u16();

                    if status >= 400 {
                        warn!(
                            method = %method,
                            path = %path,
                            query = %query,
                            status = status,
                            duration_ms = duration.as_millis(),
                            client_ip = %client_ip,
                            user_agent = %user_agent,
                            "HTTP request completed with error"
                        );
                    } else {
                        info!(
                            method = %method,
                            path = %path,
                            status = status,
                            duration_ms = duration.as_millis(),
                            client_ip = %client_ip,
                            "HTTP request completed"
                        );
                    }
                }
                Err(_) => {
                    warn!(
                        method = %method,
                        path = %path,
                        query = %query,
                        duration_ms = duration.as_millis(),
                        client_ip = %client_ip,
                        user_agent = %user_agent,
                        "HTTP request failed"
                    );
                }
            }

            result
        })
    }
}

/// 获取客户端IP地址
pub fn get_client_ip(req: &Request<Body>) -> String {
    // 优先从X-Forwarded-For头获取
    if let Some(forwarded_for) = req.headers().get("x-forwarded-for") {
        if let Ok(forwarded_str) = forwarded_for.to_str() {
            if let Some(first_ip) = forwarded_str.split(',').next() {
                return first_ip.trim().to_string();
            }
        }
    }

    // 其次从X-Real-IP头获取
    if let Some(real_ip) = req.headers().get("x-real-ip") {
        if let Ok(ip_str) = real_ip.to_str() {
            return ip_str.to_string();
        }
    }

    // 最后从连接信息获取
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// 获取User-Agent
fn get_user_agent(req: &Request<Body>) -> String {
    req.headers()
        .get("user-agent")
        .and_then(|ua| ua.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}
