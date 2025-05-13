use anyhow::Result;
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tonic::transport::{Channel, Endpoint};
use tracing::{error, info};

use crate::service_registry::ServiceRegistry;

/// gRPC服务客户端，用于调用其他微服务的gRPC接口
#[derive(Clone, Debug)]
pub struct GrpcServiceClient {
    service_registry: ServiceRegistry,
    service_name: String,
    // 缓存已发现的服务Channel
    channels: Arc<Mutex<Vec<Channel>>>,
    // 配置参数
    connection_timeout: Duration,
    request_timeout: Duration,
    concurrency_limit: usize,
}

impl GrpcServiceClient {
    /// 创建新的gRPC服务客户端
    pub fn new(
        service_registry: ServiceRegistry,
        service_name: &str,
        connection_timeout: Duration,
        request_timeout: Duration,
        concurrency_limit: usize,
    ) -> Self {
        Self {
            service_registry,
            service_name: service_name.to_string(),
            channels: Arc::new(Mutex::new(Vec::new())),
            connection_timeout,
            request_timeout,
            concurrency_limit,
        }
    }

    /// 使用默认设置创建新的gRPC服务客户端
    pub fn with_defaults(service_registry: ServiceRegistry, service_name: &str) -> Self {
        Self::new(
            service_registry,
            service_name,
            Duration::from_secs(5),
            Duration::from_secs(30),
            100,
        )
    }

    /// 从环境变量创建服务客户端
    pub fn from_env(service_name: &str) -> Self {
        let service_registry = ServiceRegistry::from_env();
        Self::with_defaults(service_registry, service_name)
    }

    /// 刷新服务通道
    pub async fn refresh_channels(&self) -> Result<()> {
        // 从Consul获取服务实例
        let service_urls = self
            .service_registry
            .discover_service(&self.service_name)
            .await?;

        if service_urls.is_empty() {
            return Err(anyhow::anyhow!(
                "没有发现可用的 {} 服务实例",
                self.service_name
            ));
        }

        // 创建新的gRPC通道
        let mut new_channels = Vec::with_capacity(service_urls.len());
        for url in service_urls {
            // 转换HTTP URL到gRPC URL (移除http:// 前缀)
            let grpc_url = if url.starts_with("http://") {
                url[7..].to_string()
            } else {
                url
            };

            match self.create_channel(&grpc_url).await {
                Ok(channel) => {
                    new_channels.push(channel);
                }
                Err(err) => {
                    error!("无法连接到gRPC服务 {}: {}", grpc_url, err);
                }
            }
        }

        if new_channels.is_empty() {
            return Err(anyhow::anyhow!(
                "没有可用的 {} 服务实例连接",
                self.service_name
            ));
        }

        // 更新通道缓存
        let mut channels = self.channels.lock().await;
        *channels = new_channels;

        info!(
            "已更新 {} 服务的 {} 个gRPC连接",
            self.service_name,
            channels.len()
        );
        Ok(())
    }

    /// 创建单个gRPC通道
    async fn create_channel(&self, target: &str) -> Result<Channel, tonic::transport::Error> {
        let endpoint = Endpoint::from_shared(format!("http://{}", target))?
            .connect_timeout(self.connection_timeout)
            .timeout(self.request_timeout)
            .concurrency_limit(self.concurrency_limit);

        endpoint.connect().await
    }

    /// 获取通道（带负载均衡）
    pub async fn get_channel(&self) -> Result<Channel> {
        // 检查缓存是否为空
        {
            let channels = self.channels.lock().await;
            if !channels.is_empty() {
                // 简单轮询负载均衡
                let index = rand::rng().random_range(0..channels.len());
                return Ok(channels[index].clone());
            }
        }

        // 缓存为空，刷新通道
        self.refresh_channels().await?;

        let channels = self.channels.lock().await;
        if channels.is_empty() {
            return Err(anyhow::anyhow!("没有可用的 {} 服务实例", self.service_name));
        }

        let index = rand::rng().random_range(0..channels.len());
        Ok(channels[index].clone())
    }

    /// 启动一个后台任务定期刷新服务实例列表
    pub fn start_refresh_task(client: Arc<Self>) {
        let refresh_interval = std::env::var("SERVICE_REFRESH_INTERVAL")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30); // 默认30秒刷新一次

        let refresh_duration = Duration::from_secs(refresh_interval);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(refresh_duration);

            loop {
                interval.tick().await;

                if let Err(err) = client.refresh_channels().await {
                    error!("刷新服务实例失败: {}", err);
                }
            }
        });
    }
}

/// gRPC服务客户端工厂，用于创建各种服务的gRPC客户端
#[derive(Clone)]
pub struct GrpcClientFactory {
    service_registry: ServiceRegistry,
}

impl GrpcClientFactory {
    /// 创建新的gRPC客户端工厂
    pub fn new(service_registry: ServiceRegistry) -> Self {
        Self { service_registry }
    }

    /// 从环境变量创建gRPC客户端工厂
    pub fn from_env() -> Self {
        let service_registry = ServiceRegistry::from_env();
        Self::new(service_registry)
    }

    /// 创建指定服务的gRPC客户端
    pub fn create_client(&self, service_name: &str) -> GrpcServiceClient {
        GrpcServiceClient::with_defaults(self.service_registry.clone(), service_name)
    }

    /// 创建指定服务的gRPC客户端，带自定义配置
    pub fn create_client_with_config(
        &self,
        service_name: &str,
        connection_timeout: Duration,
        request_timeout: Duration,
        concurrency_limit: usize,
    ) -> GrpcServiceClient {
        GrpcServiceClient::new(
            self.service_registry.clone(),
            service_name,
            connection_timeout,
            request_timeout,
            concurrency_limit,
        )
    }
}