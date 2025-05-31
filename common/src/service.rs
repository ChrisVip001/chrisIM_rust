//! 🚀 统一服务模块 - 服务注册、发现、客户端一体化
//! 
//! 这个模块统一管理：
//! 1. 服务注册 - 将服务注册到注册中心
//! 2. 服务发现 - 从注册中心发现服务
//! 3. 客户端获取 - 基于服务发现创建gRPC客户端
//! 
//! ## 核心设计理念
//! - **统一接口**: 一个模块处理所有服务相关操作
//! - **零配置**: 自动读取全局配置，无需手动传参
//! - **智能缓存**: 自动缓存客户端和连接，避免重复创建
//! - **自动重试**: 内置重试机制，提高可靠性
//! - **类型安全**: 编译时检查，减少运行时错误

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use anyhow::Result;
use tokio::sync::RwLock;
use tonic::transport::Channel;

use crate::config::{AppConfig, ConfigLoader, Component};
use crate::Error;
use crate::service_register_center::{ServiceRegister, service_register_center, typos::Registration};

// ============================================================================
// 全局状态管理（简化版）
// ============================================================================

/// 全局服务注册中心
static REGISTRY: OnceLock<std::sync::Arc<dyn ServiceRegister>> = OnceLock::new();

/// 全局Channel缓存 - 避免重复创建连接
static CHANNELS: OnceLock<RwLock<HashMap<String, Channel>>> = OnceLock::new();

/// 全局服务ID缓存 - 记录已注册的服务
static REGISTERED_SERVICES: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

// ============================================================================
// 初始化 - 应用启动时调用（大幅简化）
// ============================================================================

/// 🎯 初始化服务模块（使用全局配置中心）
/// 
/// **注意**: 调用此函数前必须确保 `ConfigLoader::set_global()` 已经调用
/// 
/// ```rust
/// use common::service;
/// use common::config::ConfigLoader;
///
/// #[tokio::main]
/// async fn main() -> Result<(),Error> {
///     // 1. 设置全局配置（必须先调用）
///     use common::config::AppConfig;
/// use common::Error;
/// let config = AppConfig::from_file(Some("./config/config.yaml"))?;
///     ConfigLoader::set_global(config);
///     
///     // 2. 初始化服务模块（不再需要传参）
///     service::init();
///     
///     // 现在可以在任何地方使用服务
///     let user_client = service::user_client().await?;
///     Ok(())
/// }
/// ```
pub fn init() {
    let config = ConfigLoader::get_global()
        .expect("全局配置未初始化，请先调用 ConfigLoader::set_global()");
    
    REGISTRY.set(service_register_center(&config)).ok();
    CHANNELS.set(RwLock::new(HashMap::new())).ok();
    REGISTERED_SERVICES.set(RwLock::new(HashMap::new())).ok();
}

// ============================================================================
// 服务注册 - 将服务注册到注册中心
// ============================================================================

/// 🎯 注册服务到注册中心
/// 
/// 自动从全局配置读取服务信息，无需手动构造Registration
pub async fn register(component: Component) -> Result<String, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let registry = REGISTRY.get()
        .ok_or_else(|| Error::Internal("服务注册中心未初始化，请先调用 service::init()".to_string()))?;

    let registration = build_registration(&config, component.clone())?;
    let service_id = registry.register(registration).await?;
    
    // 缓存已注册的服务
    {
        let mut cache = REGISTERED_SERVICES.get().unwrap().write().await;
        cache.insert(component.to_string(), service_id.clone());
    }
    
    Ok(service_id)
}

/// 🎯 注销服务
pub async fn deregister(service_id: &str) -> Result<(), Error> {
    let registry = REGISTRY.get().unwrap();
    registry.deregister(service_id).await
}

/// 🎯 注销指定组件的服务
pub async fn deregister_component(component: Component) -> Result<(), Error> {
    let cache = REGISTERED_SERVICES.get().unwrap().read().await;
    if let Some(service_id) = cache.get(&component.to_string()) {
        deregister(service_id).await
    } else {
        Ok(()) // 服务未注册，直接返回成功
    }
}

// ============================================================================
// 服务发现 + 客户端获取 - 核心功能
// ============================================================================

/// 🎯 获取任意服务的gRPC通道
/// 
/// 这是核心函数，自动处理：
/// - 服务发现
/// - 负载均衡
/// - 连接缓存
/// - 自动重试
pub async fn channel(service_name: &str) -> Result<Channel, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let registry = REGISTRY.get().unwrap();
    let channels_cache = CHANNELS.get().unwrap();

    // 检查缓存
    {
        let cache = channels_cache.read().await;
        if let Some(ch) = cache.get(service_name) {
            return Ok(ch.clone());
        }
    }

    // 服务发现 + 重试机制
    let mut services = HashMap::new();
    for attempt in 0..5 {
        match registry.find_by_name(service_name).await {
            Ok(found_services) if !found_services.is_empty() => {
                services = found_services;
                break;
            }
            Ok(_) if attempt < 4 => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Ok(_) => return Err(Error::NotFound(service_name.to_string())),
            Err(_e) if attempt < 4 => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    // 构建负载均衡通道
    let endpoints = services.values().map(|registration| {
        let url = format!("{}://{}:{}", 
            config.service_center.protocol, 
            registration.host, 
            registration.port
        );
        tonic::transport::Endpoint::from_shared(url).unwrap()
    });

    let channel = Channel::balance_list(endpoints);

    // 缓存通道
    {
        let mut cache = channels_cache.write().await;
        cache.insert(service_name.to_string(), channel.clone());
    }

    Ok(channel)
}

// ============================================================================
// 便利函数 - 直接获取各种客户端
// ============================================================================

/// 🎯 获取用户服务客户端
pub async fn user_client() -> Result<crate::proto::user::user_service_client::UserServiceClient<Channel>, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let ch = channel(&config.rpc.user.name).await?;
    Ok(crate::proto::user::user_service_client::UserServiceClient::new(ch))
}

/// 🎯 获取好友服务客户端
pub async fn friend_client() -> Result<crate::proto::friend::friend_service_client::FriendServiceClient<Channel>, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let ch = channel(&config.rpc.friend.name).await?;
    Ok(crate::proto::friend::friend_service_client::FriendServiceClient::new(ch))
}

/// 🎯 获取群组服务客户端
pub async fn group_client() -> Result<crate::proto::group::group_service_client::GroupServiceClient<Channel>, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let ch = channel(&config.rpc.group.name).await?;
    Ok(crate::proto::group::group_service_client::GroupServiceClient::new(ch))
}

/// 🎯 获取聊天服务客户端
pub async fn chat_client() -> Result<crate::message::chat_service_client::ChatServiceClient<Channel>, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let ch = channel(&config.rpc.chat.name).await?;
    Ok(crate::message::chat_service_client::ChatServiceClient::new(ch))
}

// ============================================================================
// 内部辅助函数
// ============================================================================

/// 根据组件类型构建服务注册信息
fn build_registration(config: &Arc<AppConfig>, component: Component) -> Result<Registration, Error> {
    let (name, host, port, tags, health_check) = match component {
        Component::UserServer => {
            let user_config = &config.rpc.user;
            (
                user_config.name.clone(),
                user_config.host.clone(),
                user_config.port,
                user_config.tags.clone(),
                user_config.health_check.as_ref().map(|hc| crate::service_register_center::typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                })
            )
        }
        Component::FriendServer => {
            let friend_config = &config.rpc.friend;
            (
                friend_config.name.clone(),
                friend_config.host.clone(),
                friend_config.port,
                friend_config.tags.clone(),
                friend_config.health_check.as_ref().map(|hc| crate::service_register_center::typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                })
            )
        }
        Component::GroupServer => {
            let group_config = &config.rpc.group;
            (
                group_config.name.clone(),
                group_config.host.clone(),
                group_config.port,
                group_config.tags.clone(),
                group_config.health_check.as_ref().map(|hc| crate::service_register_center::typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                })
            )
        }
        Component::MessageServer => {
            let chat_config = &config.rpc.chat;
            (
                chat_config.name.clone(),
                chat_config.host.clone(),
                chat_config.port,
                chat_config.tags.clone(),
                chat_config.health_check.as_ref().map(|hc| crate::service_register_center::typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                })
            )
        }
        Component::MessageGateway => {
            let ws_config = &config.rpc.ws;
            (
                ws_config.name.clone(),
                ws_config.host.clone(),
                ws_config.port,
                ws_config.tags.clone(),
                ws_config.health_check.as_ref().map(|hc| crate::service_register_center::typos::HealthCheck {
                    health_type: hc.health_type.clone(),
                    name: hc.name.clone(),
                    url: hc.url.clone(),
                    interval: hc.interval.to_string(),
                    timeout: hc.timeout.to_string(),
                    deregister_after: hc.deregister_after.to_string(),
                })
            )
        }
        Component::All => {
            return Err(Error::Internal("不支持注册所有服务，请逐个注册".to_string()));
        }
    };

    Ok(Registration {
        id: format!("{}-{}-{}", name, host, port),
        name,
        host,
        port,
        tags,
        check: health_check,
    })
}

// ============================================================================
// Component扩展方法
// ============================================================================

impl Component {
    fn to_string(&self) -> String {
        match self {
            Component::UserServer => "user-server".to_string(),
            Component::FriendServer => "friend-server".to_string(),
            Component::GroupServer => "group-server".to_string(),
            Component::MessageServer => "message-server".to_string(),
            Component::MessageGateway => "message-gateway".to_string(),
            Component::All => "all".to_string(),
        }
    }
}

// ============================================================================
// 宏简化（可选）
// ============================================================================

/// 🎯 进一步简化的宏
#[macro_export]
macro_rules! service {
    (register $component:expr) => { register($component).await };
    (user) => { user_client().await };
    (friend) => { friend_client().await };
    (group) => { group_client().await };
    (chat) => { chat_client().await };
    ($service:expr) => { channel($service).await };
}

// ============================================================================
// 使用示例和对比
// ============================================================================

#[allow(dead_code)]
async fn usage_examples() -> Result<()> {
    // ✅ 统一的服务操作

    // 1. 服务注册
    let _user_service_id = register(Component::UserServer).await?;
    let _friend_service_id = register(Component::FriendServer).await?;

    // 2. 获取客户端
    let _user_client = user_client().await?;
    let _friend_client = friend_client().await?;
    let _custom_channel = channel("payment-service").await?;

    // 3. 使用宏进一步简化
    let _user = service!(user)?;
    let _custom = service!("any-service")?;

    // 4. 程序结束时注销服务
    deregister(&_user_service_id).await?;
    deregister(&_friend_service_id).await?;
    // 或者
    deregister_component(Component::UserServer).await?;

    Ok(())
}

// ============================================================================
// 高级API - 支持动态服务发现（可选）
// ============================================================================

/// 🔧 高级选项：获取带动态服务发现的通道
/// 
/// 当需要动态更新服务列表时使用此函数
pub async fn dynamic_channel(service_name: &str) -> Result<crate::service_discovery::LbWithServiceDiscovery, Error> {
    let config = ConfigLoader::get_global()
        .ok_or_else(|| Error::Internal("全局配置未初始化".to_string()))?;
    let registry = REGISTRY.get().unwrap();

    // 创建动态服务发现通道
    let (ch, sender) = Channel::balance_channel(1024);
    
    // 创建服务解析器（从service_discovery模块导入）
    let resolver = crate::service_discovery::ServiceResolver::new(
        registry.clone(), 
        service_name.to_string()
    );
    
    let mut discovery = crate::service_discovery::DynamicServiceDiscovery::new(
        resolver,
        Duration::from_secs(10),
        sender,
        config.service_center.protocol.clone(),
    );

    // 初始服务发现
    for attempt in 0..5 {
        match discovery.discovery().await {
            Ok(_) => break,
            Err(_e) if attempt < 4 => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    
    // 启动后台更新任务
    tokio::spawn(discovery.run());
    
    Ok(crate::service_discovery::LbWithServiceDiscovery(ch))
}

/// 🔧 刷新服务通道缓存
/// 
/// 手动触发服务列表更新
pub async fn refresh_channel(service_name: &str) -> Result<(), Error> {
    let channels_cache = CHANNELS.get().unwrap();
    
    // 清除指定服务的缓存
    {
        let mut cache = channels_cache.write().await;
        cache.remove(service_name);
    }
    
    // 预热新的连接
    let _ = channel(service_name).await?;
    
    Ok(())
}

/// 🔧 刷新所有服务通道缓存
/// 
/// 清空所有缓存，下次访问时重新获取服务列表
pub async fn refresh_all_channels() -> Result<(), Error> {
    let channels_cache = CHANNELS.get().unwrap();
    
    // 清空所有缓存
    {
        let mut cache = channels_cache.write().await;
        cache.clear();
    }
    
    Ok(())
}

// ============================================================================
// 工具函数 - 向后兼容
// ============================================================================

/// 🔧 初始化 rustls 加密提供程序
/// 
/// 这个函数用于初始化 rustls 的默认加密提供程序，
/// 为 TLS 连接提供加密支持。
pub fn init_rustls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 🔧 关闭信号处理
/// 
/// 监听系统关闭信号(SIGINT, SIGTERM)，并优雅关闭服务
/// 
/// ```rust
/// use tokio::sync::oneshot;
/// use common::config::Component;
/// use common::service::shutdown_signal;
/// let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
/// let shutdown_task = tokio::spawn(async move {
///     shutdown_signal(shutdown_tx, Component::UserServer).await
/// });
/// ```
pub async fn shutdown_signal(
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    component: Component,
) -> Result<()> {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("无法安装 Ctrl+C 处理器");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法安装 SIGTERM 处理器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("接收到关闭信号，开始优雅关闭...");

    // 注销服务
    deregister_component(component).await?;

    // 发送关闭信号
    if let Err(_) = shutdown_tx.send(()) {
        tracing::warn!("无法发送关闭信号，接收端可能已关闭");
    }

    Ok(())
}
