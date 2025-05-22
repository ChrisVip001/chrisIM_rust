use std::collections::HashSet;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use tokio::sync::mpsc;
use tonic::body::BoxBody;
use tonic::client::GrpcService;
use tonic::transport::{Channel, Endpoint};
use tower::discover::Change;
use tracing::{error, warn};

use crate::Error;

use crate::service_discovery::service_fetcher::ServiceFetcher;

/// 自定义负载均衡器
/// 
/// 包装 tonic Channel，提供服务发现和负载均衡功能
#[derive(Debug, Clone)]
pub struct LbWithServiceDiscovery(pub Channel);

/// 为自定义负载均衡器实现 tower 服务特征
///
/// 这使得 LbWithServiceDiscovery 可以被用作 gRPC 客户端通道
impl tower::Service<http::Request<BoxBody>> for LbWithServiceDiscovery {
    type Response = http::Response<<Channel as GrpcService<BoxBody>>::ResponseBody>;
    type Error = <Channel as GrpcService<BoxBody>>::Error;
    type Future = <Channel as GrpcService<BoxBody>>::Future;

    /// 检查服务是否准备好处理请求
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        GrpcService::poll_ready(&mut self.0, cx)
    }

    /// 处理请求
    fn call(&mut self, request: http::Request<BoxBody>) -> Self::Future {
        GrpcService::call(&mut self.0, request)
    }
}

/// 动态服务发现实现
///
/// 负责定期从服务注册中心获取服务列表，并更新负载均衡器
pub struct DynamicServiceDiscovery<Fetcher: ServiceFetcher> {
    // 当前已知的服务地址集合
    services: HashSet<SocketAddr>,
    // 用于发送服务变更通知的 mpsc 发送端
    sender: mpsc::Sender<Change<SocketAddr, Endpoint>>,
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
        sender: mpsc::Sender<Change<SocketAddr, Endpoint>>,
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
    ) -> Vec<Change<SocketAddr, Endpoint>> {
        let mut changes = Vec::new();
        // 添加新增的服务
        for s in endpoints.difference(&self.services) {
            if let Some(endpoint) = self.build_endpoint(*s).await {
                changes.push(Change::Insert(*s, endpoint));
            }
        }
        // 移除不再存在的服务
        for s in self.services.difference(endpoints) {
            changes.push(Change::Remove(*s));
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

    /// 运行服务发现循环
    ///
    /// 按设定的间隔时间定期执行服务发现
    pub async fn run(mut self) {
        loop {
            tokio::time::sleep(self.dis_interval).await;
            // 从服务注册中心获取服务
            if let Err(e) = self.discovery().await {
                error!("服务发现错误:{:?}", e);
            }
        }
    }
}
