use std::collections::HashSet;
use std::future::Future;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use tokio::sync::mpsc;
use tonic::client::GrpcService;
use tonic::transport::{Channel, Endpoint};
use tracing::{error, warn};

use contract::Error;

use crate::service_discovery::service_fetcher::ServiceFetcher;

/// 自定义负载均衡器
///
/// 包装 tonic Channel，提供服务发现和负载均衡功能
pub struct LbWithServiceDiscovery {
    inner: Channel,
    /// 本通道生效的调用超时，构造时取全局默认值
    call_timeout: std::time::Duration,
    /// 等待就绪的截止计时器：无可用实例时通道永远 Pending，靠它兜底
    ready_deadline: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}

impl Clone for LbWithServiceDiscovery {
    fn clone(&self) -> Self {
        // 计时器只属于单次就绪等待，克隆时重置即可
        Self {
            inner: self.inner.clone(),
            call_timeout: self.call_timeout,
            ready_deadline: None,
        }
    }
}

impl std::fmt::Debug for LbWithServiceDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LbWithServiceDiscovery")
            .field("inner", &self.inner)
            .field("call_timeout", &self.call_timeout)
            .field("waiting_ready", &self.ready_deadline.is_some())
            .finish()
    }
}

impl LbWithServiceDiscovery {
    /// 包装一个带服务发现负载均衡的通道，超时取当前全局默认值
    pub fn new(inner: Channel) -> Self {
        Self {
            inner,
            call_timeout: rpc_call_timeout(),
            ready_deadline: None,
        }
    }
}

/// 跨服务 gRPC 调用的整体超时秒数，默认 5 秒
///
/// 服务发现没有可用实例时，tonic 的负载均衡通道不会返回错误，
/// 而是让 `poll_ready` 永远 Pending：生成的客户端在 `ready()` 处
/// 无限等待，单个调用即可挂死整条处理链路（网关的处理循环、
/// 消费端的推送批次都会被卡住，位点也不再提交）。
/// 因此在这里统一兜底，所有经 `get_chan` 构建的客户端自动获得超时。
static RPC_CALL_TIMEOUT_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(5);

/// 当前生效的 gRPC 调用超时时长
pub fn rpc_call_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(RPC_CALL_TIMEOUT_SECS.load(std::sync::atomic::Ordering::Relaxed))
}

/// 调整 gRPC 调用超时（秒），主要供运维与测试使用
pub fn set_rpc_call_timeout(secs: u64) {
    RPC_CALL_TIMEOUT_SECS.store(secs, std::sync::atomic::Ordering::Relaxed);
}

fn unavailable_status(timeout: std::time::Duration) -> tonic::Status {
    tonic::Status::unavailable(format!(
        "跨服务gRPC等待超时(>{:?}): 下游无可用实例或未响应",
        timeout
    ))
}

/// 为自定义负载均衡器实现 tower 服务特征
///
/// 这使得 LbWithServiceDiscovery 可以被用作 gRPC 客户端通道。
/// 两处超时兜底：
/// - `poll_ready`：无可用实例时通道永远 Pending，用有状态的截止计时器兜底
///   （Sleep 的 waker 注册进调用方，到期会唤醒重新 poll 并返回错误）；
/// - `call`：请求已发出但下游迟迟不响应时整体超时。
impl tower::Service<http::Request<tonic::body::Body>> for LbWithServiceDiscovery {
    type Response = http::Response<<Channel as GrpcService<tonic::body::Body>>::ResponseBody>;
    type Error = tonic::Status;
    type Future = std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<<Channel as GrpcService<tonic::body::Body>>::ResponseBody>,
                        tonic::Status,
                    >,
                > + Send,
        >,
    >;

    /// 检查服务是否准备好处理请求
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match GrpcService::poll_ready(&mut self.inner, cx) {
            Poll::Ready(result) => {
                // 就绪或失败都结束本轮等待
                self.ready_deadline = None;
                Poll::Ready(result.map_err(|e| tonic::Status::from_error(Box::new(e))))
            }
            Poll::Pending => {
                let timeout = self.call_timeout;
                let deadline = self
                    .ready_deadline
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
                if deadline.as_mut().poll(cx).is_ready() {
                    self.ready_deadline = None;
                    return Poll::Ready(Err(unavailable_status(timeout)));
                }
                Poll::Pending
            }
        }
    }

    /// 处理请求
    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        // 约定就绪后才调用；进入调用阶段，等待计时器随之结束
        self.ready_deadline = None;
        let inner = GrpcService::call(&mut self.inner, request);
        let timeout = self.call_timeout;
        Box::pin(async move {
            match tokio::time::timeout(timeout, inner).await {
                Ok(response) => response.map_err(|e| tonic::Status::from_error(Box::new(e))),
                Err(_) => Err(unavailable_status(timeout)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::Service;

    /// 复刻生成客户端的调用序列：先 `ready()`（poll_ready 循环），再 `call()`
    async fn call_like_generated_client<S>(
        svc: &mut S,
        path: &str,
    ) -> Result<(), S::Error>
    where
        S: Service<http::Request<tonic::body::Body>> + Unpin + Send,
        S::Future: Send,
    {
        std::future::poll_fn(|cx| Service::poll_ready(svc, cx)).await?;
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri(format!("http://localhost/{}", path))
            .body(tonic::body::Body::empty())
            .unwrap();
        Service::call(svc, request).await.map(|_| ())
    }

    fn channel_with_timeout(secs: u64) -> LbWithServiceDiscovery {
        let (channel, _tx) = Channel::balance_channel::<std::net::SocketAddr>(1);
        LbWithServiceDiscovery {
            inner: channel,
            call_timeout: std::time::Duration::from_secs(secs),
            ready_deadline: None,
        }
    }

    /// 超时兜底：错误码、错误信息、耗时都必须符合预期
    #[tokio::test]
    async fn call_times_out_when_no_instance_available() {
        let mut lb = channel_with_timeout(1);

        let start = std::time::Instant::now();
        // 空通道 ready 立即成功，挂死发生在 call 的响应 Future 里，由超时兜底
        let result = call_like_generated_client(&mut lb, "none.Echo/Echo").await;
        let elapsed = start.elapsed();

        // 错误要能被上层现有错误处理直接消费：
        // 客户端层错误就是 tonic::Status，网关/消费端无需任何适配
        let status = result.expect_err("无可用实例时必须返回错误而不是挂起");
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(
            status.message().contains("跨服务gRPC等待超时"),
            "错误信息: {}",
            status.message()
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(900),
            "超时太早返回: {:?}",
            elapsed
        );
        assert!(
            elapsed < std::time::Duration::from_secs(4),
            "超时未生效，调用被挂起: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn ready_succeeds_once_instance_is_registered() {
        let (channel, tx) = Channel::balance_channel::<std::net::SocketAddr>(1);
        let mut lb = LbWithServiceDiscovery {
            inner: channel,
            call_timeout: std::time::Duration::from_secs(1),
            ready_deadline: None,
        };

        // 注册一个真实监听的地址（但不是 gRPC 服务也无妨，ready 只看连接建立）
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let socket_addr: std::net::SocketAddr = addr.into();
        tx.try_send(tonic::transport::channel::Change::Insert(
            socket_addr,
            Endpoint::from_shared(format!("http://{}", addr)).unwrap(),
        ))
        .unwrap();

        // ready() 应在超时窗口内完成，而不是等满 1 秒
        let start = std::time::Instant::now();
        let result =
            std::future::poll_fn(|cx| Service::poll_ready(&mut lb, cx)).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "注册实例后应当就绪: {:?}", result.err());
        assert!(
            elapsed < std::time::Duration::from_millis(900),
            "就绪耗时异常: {:?}",
            elapsed
        );
    }
}

/// 动态服务发现实现
///
/// 负责定期从服务注册中心获取服务列表，并更新负载均衡器
pub struct DynamicServiceDiscovery<Fetcher: ServiceFetcher> {
    // 当前已知的服务地址集合
    services: HashSet<SocketAddr>,
    // 用于发送服务变更通知的 mpsc 发送端
    sender: mpsc::Sender<tower::discover::Change<SocketAddr, Endpoint>>,
    // 服务发现的间隔时间
    dis_interval: tokio::time::Duration,
    // 服务获取器，用于从服务注册中心获取服务
    service_center: Fetcher,
    // 协议模式 (http/https)
    schema: String,
}

impl<Fetcher: ServiceFetcher> DynamicServiceDiscovery<Fetcher> {
    /// 创建一个新的动态服务发现实例
    ///
    /// # 参数
    /// * `service_center` - 服务获取器
    /// * `dis_interval` - 服务发现间隔时间
    /// * `sender` - 用于发送服务变更通知的发送端
    /// * `schema` - 协议模式 (如 "http", "https")
    pub fn new(
        service_center: Fetcher,
        dis_interval: tokio::time::Duration,
        sender: mpsc::Sender<tower::discover::Change<SocketAddr, Endpoint>>,
        schema: String,
    ) -> Self {
        Self {
            services: Default::default(),
            sender,
            dis_interval,
            service_center,
            schema,
        }
    }

    /// 执行一次服务发现
    ///
    /// 从服务注册中心获取服务列表，计算变更，并更新负载均衡器
    pub async fn discovery(&mut self) -> Result<(), Error> {
        // 从服务注册中心获取服务
        let x = self.service_center.fetch().await?;
        let change_set = self.change_set(&x).await;
        for change in change_set {
            self.sender.send(change).await.map_err(|e| {
                Error::Internal(format!("发送服务变更集合错误:{:?}", e))
            })?;
        }
        self.services = x;
        Ok(())
    }

    /// 计算服务变更集合
    ///
    /// 比较当前服务集合和新获取的服务集合，生成添加和删除的变更指令
    async fn change_set(
        &self,
        endpoints: &HashSet<SocketAddr>,
    ) -> Vec<tower::discover::Change<SocketAddr, Endpoint>> {
        let mut changes = Vec::new();
        // 添加新增的服务
        for s in endpoints.difference(&self.services) {
            if let Some(endpoint) = self.build_endpoint(*s).await {
                changes.push(tower::discover::Change::Insert(*s, endpoint));
            }
        }
        // 移除不再存在的服务
        for s in self.services.difference(endpoints) {
            changes.push(tower::discover::Change::Remove(*s));
        }
        changes
    }

    /// 构建 tonic Endpoint
    ///
    /// 将服务地址转换为 tonic Endpoint 对象
    async fn build_endpoint(&self, address: SocketAddr) -> Option<Endpoint> {
        let url = format!("{}://{}:{}", self.schema, address.ip(), address.port());
        let endpoint = Endpoint::from_shared(url)
            .map_err(|e| warn!("构建端点错误:{:?}", e))
            .ok()?;
        Some(endpoint)
    }

    /// 实例集为空时的追赶刷新间隔
    ///
    /// 上游服务晚于本服务启动（或 Consul 刚重启）时，首次发现可能为空；
    /// 若仍按正常周期（30s）刷新，调用方会在空窗期内全部失败。
    /// 实例集为空时用短间隔追赶，拿到实例后恢复常规周期。
    const EMPTY_SET_REFRESH: tokio::time::Duration = tokio::time::Duration::from_secs(2);

    /// 运行服务发现循环
    ///
    /// 实例集为空时以短间隔追赶，非空时按设定间隔定期执行服务发现
    pub async fn run(mut self) {
        loop {
            let wait = if self.services.is_empty() {
                Self::EMPTY_SET_REFRESH
            } else {
                self.dis_interval
            };
            tokio::time::sleep(wait).await;
            // 从服务注册中心获取服务
            if let Err(e) = self.discovery().await {
                error!("服务发现错误:{:?}", e);
            }
        }
    }
}
