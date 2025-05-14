use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
};
use governor::clock::Clock;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tower::{BoxError, Service};
use tracing::warn;

/// 限流中间件
pub struct RateLimitLayer {
    global_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    path_limiters: Arc<
        std::collections::HashMap<String, Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>,
    >,
    ip_limiters: Arc<
        parking_lot::RwLock<
            std::collections::HashMap<
                String,
                Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
            >,
        >,
    >,
}

impl RateLimitLayer {
    /// 获取路径限流器
    fn get_path_limiter(
        &self,
        path: &str,
    ) -> Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>> {
        // 尝试匹配最长的路径前缀
        self.path_limiters
            .iter()
            .filter(|(prefix, _)| path.starts_with(*prefix))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, limiter)| limiter.clone())
    }

    /// 获取IP限流器
    fn get_ip_limiter(
        &self,
        ip: &str,
    ) -> Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>> {
        // 检查是否有针对该IP的限流器
        self.ip_limiters.read().get(ip).cloned()
    }
}

/// 限流中间件
pub struct RateLimitService<S> {
    inner: S,
    rate_limit_layer: Arc<RateLimitLayer>,
}

impl<S> RateLimitService<S> {}

impl<S> Service<Request<Body>> for RateLimitService<S>
where
    S: Service<Request<Body>, Response = Response> + Send + 'static + Clone,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // 获取请求路径
        let path = req.uri().path().to_string();

        // 获取客户端IP
        let ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // 检查全局限流
        let global_limiter = self.rate_limit_layer.global_limiter.clone();
        let global_check = global_limiter.check();

        // 检查路径限流
        let path_check = if let Some(path_limiter) = self.rate_limit_layer.get_path_limiter(&path) {
            path_limiter.check()
        } else {
            Ok(())
        };

        // 检查IP限流
        let ip_check = if let Some(ip_limiter) = self.rate_limit_layer.get_ip_limiter(&ip) {
            ip_limiter.check()
        } else {
            Ok(())
        };

        let mut svc = self.inner.clone();

        Box::pin(async move {
            // 如果任何一个限流器拒绝请求，返回429错误
            if global_check.is_err() || path_check.is_err() || ip_check.is_err() {
                // 生成剩余等待时间头
                let mut headers = HeaderMap::new();
                let mut wait_time = 0;

                // 获取当前时间
                let clock = governor::clock::DefaultClock::default();

                if let Err(wait) = global_check {
                    let wait_duration = wait.wait_time_from(clock.now());
                    wait_time = std::cmp::max(wait_time, wait_duration.as_secs());
                }

                if let Err(wait) = path_check {
                    let wait_duration = wait.wait_time_from(clock.now());
                    wait_time = std::cmp::max(wait_time, wait_duration.as_secs());
                }

                if let Err(wait) = ip_check {
                    let wait_duration = wait.wait_time_from(clock.now());
                    wait_time = std::cmp::max(wait_time, wait_duration.as_secs());
                }

                if wait_time > 0 {
                    headers.insert("Retry-After", HeaderValue::from(wait_time));
                }

                warn!("请求被限流: 路径={}, IP={}", path, ip);

                // 返回429错误
                let json_response = axum::Json(json!({
                    "error": 429,
                    "message": "请求过于频繁，请稍后重试",
                    "retry_after": wait_time,
                }));

                return Ok((StatusCode::TOO_MANY_REQUESTS, headers, json_response).into_response());
            }

            // 请求通过限流检查，继续处理
            svc.call(req).await.map_err(Into::into)
        })
    }
}

impl<S> Clone for RateLimitService<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            rate_limit_layer: self.rate_limit_layer.clone(),
        }
    }
}
