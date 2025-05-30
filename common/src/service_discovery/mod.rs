mod service_fetcher;
pub(crate) mod tonic_service_discovery;

pub use service_fetcher::*;
pub use tonic_service_discovery::*;

// 从grpc_client迁移过来的ServiceResolver
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use tracing::log::warn;
use crate::Error;
use crate::service_register_center::ServiceRegister;

/// 服务解析器，用于从服务注册中心获取服务信息
/// 
/// 这是连接服务注册中心和服务发现的桥梁
pub struct ServiceResolver {
    service_name: String,
    service_center: Arc<dyn ServiceRegister>,
}

#[async_trait]
impl ServiceFetcher for ServiceResolver {
    /// 获取服务地址集合
    async fn fetch(&self) -> Result<HashSet<SocketAddr>, Error> {
        let map = self.service_center.find_by_name(&self.service_name).await?;
        let x = map
            .values()
            .filter_map(|v| match format!("{}:{}", v.host, v.port).parse() {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!("解析主机地址错误:{}", e);
                    None
                }
            })
            .collect();
        Ok(x)
    }
}

impl ServiceResolver {
    /// 创建新的服务解析器
    pub fn new(service_center: Arc<dyn ServiceRegister>, service_name: String) -> Self {
        Self {
            service_name,
            service_center,
        }
    }
}
