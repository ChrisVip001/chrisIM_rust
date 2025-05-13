use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tower::Service;
use futures::future::BoxFuture;
use parking_lot::RwLock;
use axum::{
    http::{Request, StatusCode},
    response::{Response, IntoResponse},
    body::Body,
    Json,
};
use serde_json::json;
use crate::config::CONFIG;
use tracing::{info, warn};
use crate::proxy::service_proxy::ServiceProxy;
use tower::layer::Layer;
use tower::layer::util::Identity;

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
}

impl CircuitBreaker {
    /// 创建新的熔断器
    pub fn new(service_id: &str, failure_threshold: u64, reset_timeout_secs: u64) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitBreakerState::Closed)),
            // TODO 需要根据实际情况定义连续失败次数，可以改成从配置文件中读取
            failure_count: Arc::new(RwLock::new(5)),
            // TODO 需要根据实际情况定义失败阈值，可以改成从配置文件中读取
            failure_threshold,
            reset_timeout: Duration::from_secs(reset_timeout_secs),
            last_failure_time: Arc::new(RwLock::new(Instant::now())),
            service_id: service_id.to_string(),
        }
    }
    
    /// 获取当前熔断器状态
    pub fn state(&self) -> CircuitBreakerState {
        *self.state.read()
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
                // 半开状态下的成功请求会关闭熔断器
                *state = CircuitBreakerState::Closed;
                *self.failure_count.write() = 0;
                info!("服务 {} 熔断器已关闭，服务恢复正常", self.service_id);
            }
            CircuitBreakerState::Open => {
                // 开启状态不应该有请求，这是一个异常情况
                warn!("服务 {} 熔断器在开启状态收到成功请求，可能是状态不一致", self.service_id);
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
                    *self.last_failure_time.write() = Instant::now();
                    warn!("服务 {} 熔断器已打开，连续失败 {} 次", self.service_id, *failure_count);
                }
            }
            CircuitBreakerState::HalfOpen => {
                // 半开状态下的失败请求会重新打开熔断器
                *state = CircuitBreakerState::Open;
                *self.last_failure_time.write() = Instant::now();
                warn!("服务 {} 熔断器从半开状态重新打开，服务仍不可用", self.service_id);
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
                    info!("服务 {} 熔断器切换为半开状态，尝试恢复服务", self.service_id);
                    return true; // 允许请求通过
                }
                false // 拒绝请求
            }
            CircuitBreakerState::HalfOpen => {
                // 半开状态允许少量请求通过
                true
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
}

impl<S> CircuitBreakerMiddleware<S> {
    /// 创建新的熔断中间件
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            breakers: Arc::new(RwLock::new(HashMap::new())),
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
        
        // 从配置中读取熔断参数
        let config_future = CONFIG.read();
        let config = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(config_future)
        });
        
        // 创建新的熔断器
        let breaker = Arc::new(CircuitBreaker::new(
            service_id,
            config.circuit_breaker.failure_threshold,
            config.circuit_breaker.half_open_timeout_secs,
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

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // 从请求路径或头部确定服务ID
        let service_id = extract_service_id(&req);
        let breaker = self.get_or_create_breaker(&service_id);
        
        // 检查熔断器状态
        if !breaker.check() {
            // 熔断器打开，快速失败
            let json_response = Json(json!({
                "error": 503,
                "message": "服务暂时不可用，请稍后重试",
                "service": service_id
            }));
            
            let response = (StatusCode::SERVICE_UNAVAILABLE, json_response).into_response();
            return Box::pin(async { Ok(response) });
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
                    } else {
                        // 5xx错误被视为服务端错误，触发熔断
                        if response.status().is_server_error() {
                            breaker_clone.record_failure();
                        }
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
        }
    }
}

/// 从请求中提取服务ID
fn extract_service_id(req: &Request<Body>) -> String {
    // 首先尝试从请求头中获取
    if let Some(service) = req.headers().get("X-Target-Service") {
        if let Ok(service_str) = service.to_str() {
            return service_str.to_string();
        }
    }
    
    // 否则从路径中提取
    let path = req.uri().path();
    
    // 简单的路径解析逻辑，根据路径前缀确定服务
    if path.starts_with("/api/auth") {
        "auth-service".to_string()
    } else if path.starts_with("/api/users") {
        "user-service".to_string()
    } else if path.starts_with("/api/friends") {
        "friend-service".to_string()
    } else if path.starts_with("/api/groups") {
        "group-service".to_string()
    } else {
        // 默认值
        "unknown-service".to_string()
    }
}

/// 熔断中间件层
#[derive(Clone)]
pub struct CircuitBreakerLayer;

impl CircuitBreakerLayer {
    /// 创建新的熔断层
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for CircuitBreakerLayer {
    type Service = CircuitBreakerMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CircuitBreakerMiddleware::new(inner)
    }
}

/// 创建熔断中间件层
pub async fn circuit_breaker_layer(_service_proxy: ServiceProxy) -> Identity {
    // 简单实现，返回一个恒等中间件
    Identity::new()
} 