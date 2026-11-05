use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use runtime::config::AppConfig;
use futures::future::BoxFuture;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::layer::Layer;
use tower::Service;
use tracing::{info, warn};

use crate::proxy::services::common::error_response;

/// 熔断器状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerState {
    /// 关闭状态 - 请求正常通过
    Closed,
    /// 开启状态 - 快速失败，不发送请求
    Open,
    /// 半开状态 - 允许部分请求通过以探测服务是否恢复
    HalfOpen,
}

/// 服务熔断器
#[derive(Clone)]
pub struct CircuitBreaker {
    /// 熔断器状态
    state: Arc<RwLock<CircuitBreakerState>>,
    /// 连续失败次数
    failure_count: Arc<RwLock<u64>>,
    /// 失败阈值
    failure_threshold: u64,
    /// 开启状态的重置时间
    reset_timeout: Duration,
    /// 上次状态变更时间
    last_failure_time: Arc<RwLock<Instant>>,
    /// 服务标识符
    service_id: String,
    /// 半开态同时放行的探测请求上限：超出的请求直接快速失败，不再打下游
    half_open_max_probes: u32,
    /// 半开态需要连续成功的次数，达到后才关闭熔断器
    half_open_success_threshold: u32,
    /// 半开态当前在途的探测请求数
    half_open_inflight: Arc<AtomicU32>,
    /// 半开态累计连续成功次数
    half_open_successes: Arc<AtomicU32>,
}

impl CircuitBreaker {
    /// 创建新的熔断器
    pub fn new(
        service_id: &str,
        failure_threshold: u64,
        reset_timeout_secs: u64,
        half_open_max_probes: u32,
        half_open_success_threshold: u32,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitBreakerState::Closed)),
            // 连续失败计数从 0 开始；阈值由调用方传入
            failure_count: Arc::new(RwLock::new(0)),
            failure_threshold,
            reset_timeout: Duration::from_secs(reset_timeout_secs),
            last_failure_time: Arc::new(RwLock::new(Instant::now())),
            service_id: service_id.to_string(),
            half_open_max_probes: half_open_max_probes.max(1),
            half_open_success_threshold: half_open_success_threshold.max(1),
            half_open_inflight: Arc::new(AtomicU32::new(0)),
            half_open_successes: Arc::new(AtomicU32::new(0)),
        }
    }

    /// 当前状态（只读，单测用来断言状态机）
    #[cfg(test)]
    pub fn current_state(&self) -> CircuitBreakerState {
        *self.state.read()
    }

    /// 尝试占用一个半开态探测名额；名额用尽返回 false（调用方应快速失败）
    fn try_acquire_probe(&self) -> bool {
        let mut current = self.half_open_inflight.load(Ordering::Acquire);
        loop {
            if current >= self.half_open_max_probes {
                return false;
            }
            match self.half_open_inflight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    /// 归还一个探测名额（4xx 这类「不成功也不算失败」的响应也必须归还，否则名额泄漏）
    pub fn release_probe(&self) {
        let _ = self
            .half_open_inflight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                Some(v.saturating_sub(1))
            });
    }

    /// 清空半开态的两个计数器（状态切换时统一调用，避免跨周期脏值）
    fn reset_half_open_counters(&self) {
        self.half_open_inflight.store(0, Ordering::Release);
        self.half_open_successes.store(0, Ordering::Release);
    }

    /// 记录成功请求
    pub fn record_success(&self) {
        let mut state = self.state.write();

        match *state {
            CircuitBreakerState::Closed => {
                // 重置失败计数
                *self.failure_count.write() = 0;
            }
            CircuitBreakerState::HalfOpen => {
                // 归还探测名额并累计成功次数；达到阈值才关闭，避免「一次成功就全量放开」
                self.release_probe();
                let successes = self.half_open_successes.fetch_add(1, Ordering::AcqRel) + 1;
                if successes >= self.half_open_success_threshold {
                    *state = CircuitBreakerState::Closed;
                    *self.failure_count.write() = 0;
                    self.reset_half_open_counters();
                    info!(
                        "服务 {} 熔断器已关闭（半开态连续 {} 次成功）",
                        self.service_id, successes
                    );
                } else {
                    info!(
                        "服务 {} 半开态探测成功 {}/{}",
                        self.service_id, successes, self.half_open_success_threshold
                    );
                }
            }
            CircuitBreakerState::Open => {
                // 开启状态不应该有请求，这是一个异常情况
                warn!(
                    "服务 {} 熔断器在开启状态收到成功请求，可能是状态不一致",
                    self.service_id
                );
            }
        }
    }

    /// 记录失败请求
    pub fn record_failure(&self) {
        let mut state = self.state.write();

        match *state {
            CircuitBreakerState::Closed => {
                // 增加失败计数
                let mut failure_count = self.failure_count.write();
                *failure_count += 1;

                // 如果失败计数达到阈值，打开熔断器
                if *failure_count >= self.failure_threshold {
                    *state = CircuitBreakerState::Open;
                    self.reset_half_open_counters();
                    *self.last_failure_time.write() = Instant::now();
                    warn!(
                        "服务 {} 熔断器已打开，连续失败 {} 次",
                        self.service_id, *failure_count
                    );
                }
            }
            CircuitBreakerState::HalfOpen => {
                // 半开态一次失败即回开：先记下已累计的成功数用于日志，再清空计数
                let successes = self.half_open_successes.load(Ordering::Acquire);
                self.release_probe();
                self.reset_half_open_counters();
                *state = CircuitBreakerState::Open;
                *self.last_failure_time.write() = Instant::now();
                warn!(
                    "服务 {} 熔断器从半开状态重新打开（半开态 {}/{} 成功时失败），服务仍不可用",
                    self.service_id, successes, self.half_open_success_threshold
                );
            }
            CircuitBreakerState::Open => {
                // 开启状态的失败更新失败时间
                *self.last_failure_time.write() = Instant::now();
            }
        }
    }

    /// 检查熔断器状态并进行状态转换
    pub fn check(&self) -> bool {
        let mut state = self.state.write();

        match *state {
            CircuitBreakerState::Open => {
                // 如果已经超过重置超时时间，转换为半开状态
                let last_failure = *self.last_failure_time.read();
                if last_failure.elapsed() >= self.reset_timeout {
                    *state = CircuitBreakerState::HalfOpen;
                    self.reset_half_open_counters();
                    info!(
                        "服务 {} 熔断器切换为半开状态，尝试恢复服务（探测上限 {}，需连续 {} 次成功）",
                        self.service_id, self.half_open_max_probes, self.half_open_success_threshold
                    );
                    // 本次请求本身就是第 1 个探测，需要占用名额
                    return self.try_acquire_probe();
                }
                false // 拒绝请求
            }
            CircuitBreakerState::HalfOpen => {
                // 半开状态只放行有限个探测请求；名额用尽时快速失败，避免恢复瞬间打垮下游
                let admitted = self.try_acquire_probe();
                if !admitted {
                    warn!(
                        "服务 {} 半开态探测名额已满（{}），本次请求快速失败",
                        self.service_id, self.half_open_max_probes
                    );
                }
                admitted
            }
            CircuitBreakerState::Closed => {
                // 关闭状态正常允许请求
                true
            }
        }
    }
}

/// 熔断中间件
pub struct CircuitBreakerMiddleware<S> {
    inner: S,
    breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
    config: Arc<AppConfig>,
    /// `gateway.circuit_breaker.enabled` 的快照：为 false 时整体透传，不建熔断器
    enabled: bool,
}

impl<S> CircuitBreakerMiddleware<S> {
    /// 用共享的熔断器表构造（只能经 [`CircuitBreakerLayer`] 创建）
    ///
    /// 表必须由 Layer 持有：`Router::layer` 会对每条路由各调一次 `layer()`，
    /// 表若在中间件里新建，`/api/users` 与 `/api/groups` 会各有一个「relation」
    /// 熔断器，失败次数分别累计，等于把阈值乘以路由条数。
    fn with_shared_breakers(
        inner: S,
        config: Arc<AppConfig>,
        breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
    ) -> Self {
        let enabled = config.gateway.circuit_breaker.enabled;
        Self {
            inner,
            config,
            breakers,
            enabled,
        }
    }

    /// 获取或创建服务熔断器
    fn get_or_create_breaker(&self, service_id: &str) -> Arc<CircuitBreaker> {
        let breakers = self.breakers.read();

        if let Some(breaker) = breakers.get(service_id) {
            return breaker.clone();
        }

        // 如果不存在，创建新的熔断器
        drop(breakers);
        let mut breakers = self.breakers.write();

        // 双重检查
        if let Some(breaker) = breakers.get(service_id) {
            return breaker.clone();
        }

        // 从注入的配置中读取熔断参数
        let gateway_config = &self.config.gateway;

        // 创建新的熔断器
        let breaker = Arc::new(CircuitBreaker::new(
            service_id,
            gateway_config.circuit_breaker.failure_threshold,
            gateway_config.circuit_breaker.half_open_timeout_secs,
            gateway_config.circuit_breaker.half_open_max_probes,
            gateway_config.circuit_breaker.half_open_success_threshold,
        ));

        breakers.insert(service_id.to_string(), breaker.clone());
        breaker
    }
}

impl<S> Service<Request<Body>> for CircuitBreakerMiddleware<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // 只对「真正转发到下游」的路径建熔断器：网关自己处理的路由
        // （/health、/metrics、登录登出、上传签名）与未匹配路径直接透传，
        // 免得网关自身的 5xx 把公用熔断器打开、反过来把登录也熔断掉。
        let service_id = if self.enabled {
            extract_service_id(&req)
        } else {
            None
        };

        let Some(service_id) = service_id else {
            let mut inner = self.inner.clone();
            return Box::pin(inner.call(req));
        };

        tracing::debug!(path = %req.uri().path(), breaker = service_id, "熔断键解析");

        let breaker = self.get_or_create_breaker(service_id);

        // 检查熔断器状态
        if !breaker.check() {
            // 熔断器打开，快速失败；信封与业务接口保持一致，客户端不必分辨两种错误格式
            return Box::pin(async {
                Ok(error_response(
                    "服务暂时不可用，请稍后重试",
                    StatusCode::SERVICE_UNAVAILABLE,
                ))
            });
        }

        // 克隆服务实例和熔断器，以便在异步闭包中使用
        let mut svc = self.inner.clone();
        let breaker_clone = breaker.clone();

        // 请求正常通过熔断器
        Box::pin(async move {
            match svc.call(req).await {
                Ok(response) => {
                    // 判断响应是否成功
                    if response.status().is_success() {
                        breaker_clone.record_success();
                    } else if response.status().is_server_error() {
                        // 5xx 视为服务端错误，触发熔断
                        breaker_clone.record_failure();
                    } else {
                        // 4xx 既不算成功也不算失败，但半开态占用的探测名额必须归还，
                        // 否则名额会被一直占着，恢复窗口内再也放不进新探测
                        breaker_clone.release_probe();
                    }
                    Ok(response)
                }
                Err(err) => {
                    // 请求失败
                    breaker_clone.record_failure();
                    Err(err)
                }
            }
        })
    }
}

impl<S> Clone for CircuitBreakerMiddleware<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            breakers: self.breakers.clone(),
            config: self.config.clone(),
            enabled: self.enabled,
        }
    }
}

/// 从请求路径解析下游服务名，`None` 表示这条路径不参与熔断
///
/// 映射表在 [`crate::api_utils::upstream_service`]，熔断与指标共用同一份；
/// 只看路径，不看请求头（`X-Target-Service` 之类的值由客户端控制）。
fn extract_service_id(req: &Request<Body>) -> Option<&'static str> {
    crate::api_utils::upstream_service(req.uri().path())
}

/// 熔断中间件层
///
/// 携带配置快照：熔断阈值来自配置，随 Layer 一起注入，
/// 避免中间件在执行期回读全局单例。
///
/// 熔断器表由 Layer 持有并分发给每个路由：`Router::layer` 会对**每条路由**
/// 各调一次 `layer()`，表若在中间件里新建，同一个下游就会被拆成 N 份状态。
#[derive(Clone)]
pub struct CircuitBreakerLayer {
    config: Arc<AppConfig>,
    breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
}

impl CircuitBreakerLayer {
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self {
            config,
            breakers: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<S> Layer<S> for CircuitBreakerLayer {
    type Service = CircuitBreakerMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CircuitBreakerMiddleware::with_shared_breakers(
            inner,
            self.config.clone(),
            self.breakers.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// reset_timeout=0 的熔断器：进入 Open 后下一次 check 立即切半开，方便测状态机
    fn breaker(probes: u32, success_threshold: u32) -> CircuitBreaker {
        CircuitBreaker::new("test", 1, 0, probes, success_threshold)
    }

    fn trip_open(b: &CircuitBreaker) {
        b.record_failure();
        assert_eq!(b.current_state(), CircuitBreakerState::Open);
    }

    #[test]
    fn half_open_limits_concurrent_probes() {
        let b = breaker(3, 2);
        trip_open(&b);
        // 前 3 个请求各占一个探测名额
        assert!(b.check());
        assert!(b.check());
        assert!(b.check());
        // 名额用尽，第 4 个快速失败且不打下游
        assert!(!b.check());
        assert_eq!(b.current_state(), CircuitBreakerState::HalfOpen);
    }

    #[test]
    fn half_open_closes_only_after_threshold_successes() {
        let b = breaker(3, 2);
        trip_open(&b);
        assert!(b.check());
        b.record_success();
        // 只成功 1 次，未达阈值，仍是半开
        assert_eq!(b.current_state(), CircuitBreakerState::HalfOpen);
        assert!(b.check());
        b.record_success();
        // 第 2 次成功达到阈值才关闭
        assert_eq!(b.current_state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn half_open_reopens_immediately_on_failure() {
        let b = breaker(2, 3);
        trip_open(&b);
        assert!(b.check());
        b.record_success(); // 1/3
        assert!(b.check());
        b.record_failure(); // 半开态失败 → 立即回开
        assert_eq!(b.current_state(), CircuitBreakerState::Open);

        // 成功计数已清零：重新进入半开后必须重新累计 3 次
        assert!(b.check());
        b.record_success();
        b.record_success();
        assert_eq!(b.current_state(), CircuitBreakerState::HalfOpen);
        assert!(b.check());
        b.record_success();
        assert_eq!(b.current_state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn probe_slots_are_released_by_mixed_responses() {
        let b = breaker(1, 2);
        trip_open(&b);
        // 唯一名额被占用后，其它请求快速失败
        assert!(b.check());
        assert!(!b.check());
        // 4xx 这类「不成功也不算失败」的响应由中间件归还名额
        b.release_probe();
        assert!(b.check());
    }

    #[test]
    fn closed_state_ignores_probe_budget() {
        let b = breaker(1, 1);
        // 关闭态不受探测名额限制
        assert!(b.check());
        assert!(b.check());
        assert_eq!(b.current_state(), CircuitBreakerState::Closed);
    }

    fn service_of(path: &str) -> Option<&'static str> {
        let req = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("构造请求失败");
        extract_service_id(&req)
    }

    #[test]
    fn proxied_paths_use_real_service_names() {
        // 用户/好友/群组是同一个模块化单体，必须共用一个熔断器
        assert_eq!(service_of("/api/users/registerByPhone"), Some("relation"));
        assert_eq!(service_of("/api/users"), Some("relation"));
        assert_eq!(service_of("/api/friends/1/list"), Some("relation"));
        assert_eq!(service_of("/api/groups/1/members"), Some("relation"));
        assert_eq!(service_of("/api/chat/send"), Some("chat"));
    }

    #[test]
    fn gateway_local_paths_are_not_fused() {
        // 网关自己处理的路径：熔断它们等于自伤（登录会被自己的 5xx 熔断）
        assert_eq!(service_of("/health"), None);
        assert_eq!(service_of("/metrics"), None);
        assert_eq!(service_of("/api/user/login"), None);
        assert_eq!(service_of("/api/user/logout"), None);
        assert_eq!(service_of("/api/files/upload-signature"), None);
        // 这条虽然挂在 /api/friends 前缀下，但实现是网关查 Redis
        assert_eq!(service_of("/api/friends/online-status"), None);
        assert_eq!(service_of("/nope"), None);
    }

    #[test]
    fn prefix_match_stops_at_segment_boundary() {
        // 不做段边界判断的话 /api/chatxxx 会被误判成 chat
        assert_eq!(service_of("/api/chatxxx"), None);
        assert_eq!(service_of("/api/users2"), None);
    }

    fn test_config() -> Arc<AppConfig> {
        // 用仓库里的真实配置，避免为了测试抄一份字段清单
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/config.yaml");
        Arc::new(AppConfig::from_file(Some(path)).expect("加载 config/config.yaml 失败"))
    }

    fn dummy_service() -> impl Service<Request<Body>, Response = Response, Error = std::convert::Infallible> + Clone
    {
        tower::service_fn(|_req: Request<Body>| async {
            Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
        })
    }

    #[test]
    fn layer_shares_one_breaker_table_across_routes() {
        // Router::layer 会对每条路由各调一次 layer()：表必须由 Layer 持有，
        // 否则 /api/users 与 /api/groups 会各有一个「relation」熔断器
        let layer = CircuitBreakerLayer::new(test_config());
        let a = layer.layer(dummy_service());
        let b = layer.layer(dummy_service());
        assert!(
            Arc::ptr_eq(&a.breakers, &b.breakers),
            "同一 Layer 切出的中间件必须共用熔断器表"
        );

        // 反向确认：各自带一张空表的中间件彼此独立（这正是按路由 new 出来的后果）
        let cfg = test_config();
        let c = CircuitBreakerMiddleware::with_shared_breakers(
            dummy_service(),
            cfg.clone(),
            Arc::new(RwLock::new(HashMap::new())),
        );
        let d = CircuitBreakerMiddleware::with_shared_breakers(
            dummy_service(),
            cfg,
            Arc::new(RwLock::new(HashMap::new())),
        );
        assert!(!Arc::ptr_eq(&c.breakers, &d.breakers));
    }
}
