use crate::Error;
use anyhow::Result;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};
// 导入密码散列相关依赖
use crate::grpc_client::client_factory::ClientFactory;
use async_trait::async_trait;
use tracing::log::warn;

// 从本地模块导入服务发现和错误处理相关组件
use crate::config::{AppConfig, Component};
use crate::service_discovery::{DynamicServiceDiscovery, LbWithServiceDiscovery, ServiceFetcher};

// 重新导出服务注册中心模块
pub use crate::service_register_center::{service_register_center, typos, ServiceRegister};

/// 根据服务名称获取RPC通道
pub async fn get_rpc_channel_by_name(
    config: &AppConfig,
    name: &str,
    protocol: &str,
) -> Result<Channel, Error> {
    let center = service_register_center(config);
    let mut service_list = center.find_by_name(name).await?;

    // 如果没找到服务，重试5次
    if service_list.is_empty() {
        for i in 0..5 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            service_list = center.find_by_name(name).await?;
            if !service_list.is_empty() {
                break;
            }
            if i == 5 {
                return Err(Error::NotFound(name.to_string()));
            }
        }
    }
    let endpoints = service_list.values().map(|v| {
        let url = format!("{}://{}:{}", protocol, v.host, v.port);
        Endpoint::from_shared(url).unwrap()
    });
    let channel = Channel::balance_list(endpoints);
    Ok(channel)
}

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

/// 使用配置创建带服务发现功能的通道
///
/// # 参数
/// * `config` - 应用配置
/// * `service_name` - 服务名称
/// * `protocol` - 通信协议
///
/// # 返回
/// 返回带有负载均衡和服务发现功能的通道
pub async fn get_channel_with_config(
    config: &AppConfig,
    service_name: impl ToString,
    protocol: impl ToString,
) -> Result<LbWithServiceDiscovery, Error> {
    let (channel, sender) = Channel::balance_channel(1024);
    let service_resolver =
        ServiceResolver::new(service_register_center(config), service_name.to_string());
    let discovery = DynamicServiceDiscovery::new(
        service_resolver,
        Duration::from_secs(10),
        sender,
        protocol.to_string(),
    );
    get_channel(discovery, channel).await
}

/// 使用指定的服务注册中心创建带服务发现功能的通道
///
/// # 参数
/// * `register` - 服务注册中心
/// * `service_name` - 服务名称
/// * `protocol` - 通信协议
///
/// # 返回
/// 返回带有负载均衡和服务发现功能的通道
pub async fn get_channel_with_register(
    register: Arc<dyn ServiceRegister>,
    service_name: impl ToString,
    protocol: impl ToString,
) -> Result<LbWithServiceDiscovery, Error> {
    let (channel, sender) = Channel::balance_channel(1024);
    let service_resolver = ServiceResolver::new(register, service_name.to_string());
    let discovery = DynamicServiceDiscovery::new(
        service_resolver,
        Duration::from_secs(10),
        sender,
        protocol.to_string(),
    );
    get_channel(discovery, channel).await
}

/// 内部函数，用于创建带服务发现的通道
async fn get_channel(
    mut discovery: DynamicServiceDiscovery<ServiceResolver>,
    channel: Channel,
) -> Result<LbWithServiceDiscovery, Error> {
    discovery.discovery().await?;
    tokio::spawn(discovery.run());
    Ok(LbWithServiceDiscovery(channel))
}

/// 获取带负载均衡的通道
///
/// 简化版的获取通道函数，使用应用配置和服务名称
pub async fn get_chan(config: &AppConfig, name: String) -> Result<LbWithServiceDiscovery, Error> {
    let (channel, sender) = Channel::balance_channel(1024);

    // 创建 ServiceResolver
    let service_resolver = ServiceResolver::new(service_register_center(config), name.clone());

    // 创建 DynamicServiceDiscovery
    let mut discovery = DynamicServiceDiscovery::new(
        service_resolver,
        Duration::from_secs(10),
        sender,
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

    let (name, host, port, tags) = match com {
        Component::MessageServer => {
            let name = config.rpc.chat.name.clone();
            let host = config.rpc.chat.host.clone();
            let port = config.rpc.chat.port;
            let tags = config.rpc.chat.tags.clone();
            (name, host, port, tags)
        }
        Component::ApiGateway => {
            let name = config.rpc.api.name.clone();
            let host = config.rpc.api.host.clone();
            let port = config.rpc.api.port;
            let tags = config.rpc.api.tags.clone();
            (name, host, port, tags)
        }
        Component::MessageGateway => {
            let name = config.rpc.ws.name.clone();
            let host = config.rpc.ws.host.clone();
            let port = config.rpc.ws.port;
            let tags = config.rpc.ws.tags.clone();
            (name, host, port, tags)
        }
        Component::UserServer => {
            let name = config.rpc.user.name.clone();
            let host = config.rpc.user.host.clone();
            let port = config.rpc.user.port;
            let tags = config.rpc.user.tags.clone();
            (name, host, port, tags)
        }
        Component::FriendServer => {
            let name = config.rpc.friend.name.clone();
            let host = config.rpc.friend.host.clone();
            let port = config.rpc.friend.port;
            let tags = config.rpc.friend.tags.clone();
            (name, host, port, tags)
        }
        Component::GroupServer => {
            let name = config.rpc.group.name.clone();
            let host = config.rpc.group.host.clone();
            let port = config.rpc.group.port;
            let tags = config.rpc.group.tags.clone();
            (name, host, port, tags)
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
        check: None,
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
