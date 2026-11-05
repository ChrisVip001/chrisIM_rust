//! 访问日志中间件
//!
//! 日志级别按状态码区分：2xx/3xx → `debug`，4xx → `info`，5xx → `warn`。
//!
//! 只记录路径，不记录 `query`（URL 里可能带 token/手机号）；
//! IP 统一走 [`crate::api_utils::net::resolve_client_ip`]。

use axum::{
    body::Body,
    http::{Request, Response},
};
use futures::future::BoxFuture;
use std::{
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};
use tower::{Layer, Service};
use tracing::{debug, info, warn};

use crate::api_utils::net::resolve_client_ip_string;

/// 请求日志中间件层
#[derive(Clone)]
pub struct RequestLoggerLayer {
    trusted_proxies: Arc<Vec<String>>,
}

impl RequestLoggerLayer {
    pub fn new(trusted_proxies: Vec<String>) -> Self {
        Self {
            trusted_proxies: Arc::new(trusted_proxies),
        }
    }
}

impl<S> Layer<S> for RequestLoggerLayer {
    type Service = RequestLoggerMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestLoggerMiddleware {
            inner,
            trusted_proxies: self.trusted_proxies.clone(),
        }
    }
}

/// 请求日志中间件
#[derive(Clone)]
pub struct RequestLoggerMiddleware<S> {
    inner: S,
    trusted_proxies: Arc<Vec<String>>,
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
        let client_ip = resolve_client_ip_string(&req, &self.trusted_proxies);
        let user_agent = get_user_agent(&req);

        // 观测端点自身不记录，避免抓取噪音把业务请求挤出日志窗口
        let quiet = path == "/health" || path == "/metrics";

        let start = Instant::now();
        let mut svc = self.inner.clone();

        Box::pin(async move {
            let result = svc.call(req).await;
            let duration_ms = start.elapsed().as_millis();

            match &result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if quiet {
                        debug!(method = %method, path = %path, status, "HTTP request completed");
                    } else if status >= 500 {
                        warn!(
                            method = %method, path = %path, status,
                            duration_ms, client_ip = %client_ip, user_agent = %user_agent,
                            "HTTP request failed with server error"
                        );
                    } else if status >= 400 {
                        info!(
                            method = %method, path = %path, status,
                            duration_ms, client_ip = %client_ip,
                            "HTTP request rejected"
                        );
                    } else {
                        debug!(
                            method = %method, path = %path, status,
                            duration_ms, client_ip = %client_ip,
                            "HTTP request completed"
                        );
                    }
                }
                Err(_) => {
                    warn!(
                        method = %method, path = %path,
                        duration_ms, client_ip = %client_ip, user_agent = %user_agent,
                        "HTTP request failed before producing a response"
                    );
                }
            }

            result
        })
    }
}

/// 获取User-Agent
fn get_user_agent(req: &Request<Body>) -> String {
    req.headers()
        .get("user-agent")
        .and_then(|ua| ua.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}
