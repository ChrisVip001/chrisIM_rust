use crate::Error;
use anyhow::Result;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};
use tonic::transport::channel::Change as TonicChange;
// 导入密码散列相关依赖
use crate::grpc_client::client_factory::ClientFactory;
use async_trait::async_trait;
use tracing::log::warn;

// 从本地模块导入服务发现和错误处理相关组件
use crate::config::{AppConfig, Component};
use crate::service_discovery::{DynamicServiceDiscovery, LbWithServiceDiscovery, ServiceFetcher};

// 重新导出服务注册中心模块
pub use crate::service_register_center::{service_register_center, typos, ServiceRegister};

/// 服务解析器，用于从服务注册中心获取服务信息
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

///  服务解析器，用于从服务注册中心获取服务信息
impl ServiceResolver {
    /// 创建新的服务解析器
    pub fn new(service_center: Arc<dyn ServiceRegister>, service_name: String) -> Self {
        Self {
            service_name,
            service_center,
        }
    }
}

/// 获取带负载均衡的通道
///
/// 简化版的获取通道函数，使用应用配置和服务名称
pub async fn get_chan(config: &AppConfig, name: String) -> Result<LbWithServiceDiscovery, Error> {
    let (channel, tonic_sender) = Channel::balance_channel(1024);

    // 创建一个 tower::discover::Change 的转换器
    let (tower_sender, mut tower_receiver) = tokio::sync::mpsc::channel::<tower::discover::Change<SocketAddr, Endpoint>>(1024);
    
    // 启动一个任务来转换 tower::discover::Change 到 tonic Change
    let tonic_sender_clone = tonic_sender.clone();
    tokio::spawn(async move {
        while let Some(change) = tower_receiver.recv().await {
            // 使用正确的 tonic Change 类型
            let tonic_change = match change {
                tower::discover::Change::Insert(key, value) => {
                    TonicChange::Insert(key, value)
                }
                tower::discover::Change::Remove(key) => {
                    TonicChange::Remove(key)
                }
            };
            
            if let Err(_) = tonic_sender_clone.send(tonic_change).await {
                break;
            }
        }
    });

    // 创建 ServiceResolver
    let service_resolver = ServiceResolver::new(service_register_center(config), name.clone());

    // 创建 DynamicServiceDiscovery，使用 tower_sender
    let mut discovery = DynamicServiceDiscovery::new(
        service_resolver,
        Duration::from_secs(30),
        tower_sender,
        config.service_center.protocol.clone(),
    );

    // 初始化并启动服务发现
    discovery.discovery().await?;
    tokio::spawn(discovery.run());

    Ok(LbWithServiceDiscovery(channel))
}

/// 注册微服务到服务注册中心
///
/// # 参数
/// * `config` - 应用配置
/// * `com` - 服务组件类型
///
/// # 返回
/// 成功返回 Ok(()), 失败返回 Error
pub async fn register_service(config: &AppConfig, com: Component) -> Result<String, Error> {
    // 获取服务注册中心
    let service_registry = service_register_center(config);

    let (name, host, port, tags, health_check) = match com {
        Component::MessageServer => {
            let name = config.rpc.chat.name.clone();
            let host = config.rpc.chat.host.clone();
            let port = config.rpc.chat.port;
            let tags = config.rpc.chat.tags.clone();
            let health_check = config.rpc.chat.health_check.as_ref().map(|hc| {
                typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                }
            });
            (name, host, port, tags, health_check)
        }
        Component::MessageGateway => {
            let name = config.rpc.ws.name.clone();
            let host = config.rpc.ws.host.clone();
            let port = config.rpc.ws.port;
            let tags = config.rpc.ws.tags.clone();
            let health_check = config.rpc.ws.health_check.as_ref().map(|hc| {
                typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                }
            });
            (name, host, port, tags, health_check)
        }
        Component::UserServer => {
            let name = config.rpc.user.name.clone();
            let host = config.rpc.user.host.clone();
            let port = config.rpc.user.port;
            let tags = config.rpc.user.tags.clone();
            let health_check = config.rpc.user.health_check.as_ref().map(|hc| {
                typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                }
            });
            (name, host, port, tags, health_check)
        }
        Component::FriendServer => {
            let name = config.rpc.friend.name.clone();
            let host = config.rpc.friend.host.clone();
            let port = config.rpc.friend.port;
            let tags = config.rpc.friend.tags.clone();
            let health_check = config.rpc.friend.health_check.as_ref().map(|hc| {
                typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                }
            });
            (name, host, port, tags, health_check)
        }
        Component::GroupServer => {
            let name = config.rpc.group.name.clone();
            let host = config.rpc.group.host.clone();
            let port = config.rpc.group.port;
            let tags = config.rpc.group.tags.clone();
            let health_check = config.rpc.group.health_check.as_ref().map(|hc| {
                typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                }
            });
            (name, host, port, tags, health_check)
        }
        Component::All => {
            // TODO 要完善
            return Err(Error::Internal("不支持注册所有服务".to_string()));
        }
    };

    // 构建服务注册信息
    let registration = typos::Registration {
        id: format!("{}-{}-{}", name, host, port),
        name,
        host,
        port,
        tags,
        check: health_check,
    };

    // 注册服务
    let service_id = service_registry.register(registration).await?;
    Ok(service_id)
}

/// 获取RPC客户端
///
/// 使用泛型参数T，T必须实现ClientFactory特征
///
/// # 参数
/// * `config` - 应用配置
/// * `service_name` - 服务名称
///
/// # 返回
/// 返回对应类型的RPC客户端
pub async fn get_rpc_client<T: ClientFactory>(
    config: &AppConfig,
    service_name: String,
) -> Result<T, Error> {
    let channel = get_chan(config, service_name).await?;
    Ok(T::n(channel))
}
