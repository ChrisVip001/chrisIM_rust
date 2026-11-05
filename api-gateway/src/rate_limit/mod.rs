use axum::{
    body::Body,
    http::{HeaderValue, Request},
    response::{IntoResponse, Response},
};
use runtime::configs::rate_limit_config::RateLimitConfig;
use governor::clock::Clock;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tower::{layer::Layer, BoxError, Service};
use tracing::{info, warn};

use crate::api_utils::net::resolve_client_ip_string;
use crate::api_utils::path_under;
use crate::error::GatewayError;

/// 限流中间件
#[derive(Clone)]
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
    /// 传给 IP 解析的受信代理列表（见 `api_utils::net`）
    trusted_proxies: Arc<Vec<String>>,
}

/// 根据限流规则构建配额
///
/// 配置里写 0 会被夹到 1（宁可限得死一点，也不让 `NonZeroU32` 的构造 panic）
fn build_quota(rule: &runtime::configs::rate_limit_config::RateLimitRule) -> Quota {
    let per_second =
        NonZeroU32::new(rule.requests_per_second.max(1)).unwrap_or(NonZeroU32::MIN);
    let burst = NonZeroU32::new(rule.burst_size.max(1)).unwrap_or(NonZeroU32::MIN);
    Quota::per_second(per_second).allow_burst(burst)
}

impl RateLimitLayer {
    /// 根据限流配置构建限流中间件层
    pub fn new(config: &RateLimitConfig, trusted_proxies: Vec<String>) -> Self {
        let global_limiter = Arc::new(RateLimiter::direct(build_quota(&config.global)));

        let mut path_limiters = std::collections::HashMap::new();
        for rule in &config.path_rules {
            if rule.rule.enabled {
                path_limiters.insert(
                    rule.path_prefix.clone(),
                    Arc::new(RateLimiter::direct(build_quota(&rule.rule))),
                );
            }
        }

        let mut ip_map = std::collections::HashMap::new();
        for (ip, rule) in &config.ip_rules {
            if rule.enabled {
                ip_map.insert(ip.clone(), Arc::new(RateLimiter::direct(build_quota(rule))));
            }
        }

        info!(
            "限流中间件已初始化: 全局={} rps, 路径规则={} 条, IP规则={} 条",
            config.global.requests_per_second,
            path_limiters.len(),
            ip_map.len()
        );

        Self {
            global_limiter,
            path_limiters: Arc::new(path_limiters),
            ip_limiters: Arc::new(parking_lot::RwLock::new(ip_map)),
            trusted_proxies: Arc::new(trusted_proxies),
        }
    }

    /// 获取路径限流器（最长前缀，且按路径段边界匹配）
    fn get_path_limiter(
        &self,
        path: &str,
    ) -> Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>> {
        self.path_limiters
            .iter()
            .filter(|(prefix, _)| path_under(path, prefix))
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

/// 等待时间向上取整到秒：亚秒级的等待也要给出 `Retry-After: 1`，
/// 否则头被省掉，客户端会立刻重试并继续被拒。
fn wait_secs(wait: std::time::Duration) -> u64 {
    wait.as_millis().div_ceil(1000).max(1) as u64
}

/// 限流中间件
pub struct RateLimitService<S> {
    inner: S,
    rate_limit_layer: Arc<RateLimitLayer>,
}

impl<S> RateLimitService<S> {}

impl<S> Service<Request<Body>> for RateLimitService<S>
where
    S: Service<Request<Body>, Response = Response, Error = std::convert::Infallible>
        + Send
        + 'static
        + Clone,
    S::Future: Send + 'static,
{
    type Response = Response;
    // Router 的 Service 错误类型为 Infallible，错误必须折叠为响应
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // 获取请求路径
        let path = req.uri().path().to_string();

        // 获取客户端IP（受信代理由配置决定是否采信 XFF）
        let ip = resolve_client_ip_string(&req, &self.rate_limit_layer.trusted_proxies);

        // 依次检查全局 → 路径 → IP：一旦某一层拒绝，就不再消耗后面层的配额
        let mut wait_time = 0u64;
        let mut rejected_by = None;

        let clock = DefaultClock::default();
        if let Err(not_until) = self.rate_limit_layer.global_limiter.check() {
            wait_time = wait_secs(not_until.wait_time_from(clock.now()));
            rejected_by = Some("global");
        } else if let Some(limiter) = self.rate_limit_layer.get_path_limiter(&path) {
            if let Err(not_until) = limiter.check() {
                wait_time = wait_secs(not_until.wait_time_from(clock.now()));
                rejected_by = Some("path");
            }
        }

        if rejected_by.is_none() {
            if let Some(limiter) = self.rate_limit_layer.get_ip_limiter(&ip) {
                if let Err(not_until) = limiter.check() {
                    wait_time = wait_secs(not_until.wait_time_from(clock.now()));
                    rejected_by = Some("ip");
                }
            }
        }

        let mut svc = self.inner.clone();

        Box::pin(async move {
            if let Some(scope) = rejected_by {
                warn!(path = %path, ip = %ip, scope, "请求被限流");

                // 限流响应走统一错误信封，并按需带 Retry-After
                let mut response =
                    GatewayError::too_many_requests("请求过于频繁，请稍后重试").into_response();
                if wait_time > 0 {
                    response
                        .headers_mut()
                        .insert("Retry-After", HeaderValue::from(wait_time));
                }
                return Ok(response);
            }

            // 请求通过限流检查，继续处理
            match svc.call(req).await {
                Ok(resp) => Ok(resp),
                // Router 的错误类型为 Infallible，不可能出现
                Err(infallible) => match infallible {},
            }
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

impl<S> Layer<S> for RateLimitLayer
where
    S: Service<Request<Body>, Response = Response> + Send + 'static + Clone,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
{
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            rate_limit_layer: Arc::new(self.clone()),
        }
    }
}
