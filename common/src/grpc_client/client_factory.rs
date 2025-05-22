// 导入聊天服务和消息服务的客户端
use crate::message::{chat_service_client::ChatServiceClient, msg_service_client::MsgServiceClient};
use crate::proto::friend::friend_service_client::FriendServiceClient;
use crate::proto::group::group_service_client::GroupServiceClient;
use crate::proto::user::user_service_client::UserServiceClient;
// 导入自定义的服务发现负载均衡实现
use crate::service_discovery::tonic_service_discovery::LbWithServiceDiscovery;

/// 客户端工厂特征
///
/// 定义了创建RPC客户端的通用接口
/// 使不同类型的客户端能够通过统一的方法创建
pub trait ClientFactory {
    /// 创建一个新的客户端实例
    /// 
    /// # 参数
    /// * `channel` - 带负载均衡的通道
    fn n(channel: LbWithServiceDiscovery) -> Self;
}

/// 聊天服务客户端的工厂实现
impl ClientFactory for ChatServiceClient<LbWithServiceDiscovery> {
    fn n(channel: LbWithServiceDiscovery) -> Self {
        // 使用通道创建新的聊天服务客户端
        Self::new(channel)
    }
}

/// 消息服务客户端的工厂实现
impl ClientFactory for MsgServiceClient<LbWithServiceDiscovery> {
    fn n(channel: LbWithServiceDiscovery) -> Self {
        // 使用通道创建新的消息服务客户端
        Self::new(channel)
    }
}

/// 用户服务客户端的工厂实现
impl ClientFactory for UserServiceClient<LbWithServiceDiscovery>{
    fn n(channel: LbWithServiceDiscovery) -> Self {
        // 使用通道创建新的用户服务客户端
        Self::new(channel)
    }
}

/// 好友服务客户端工厂实现
impl ClientFactory for FriendServiceClient<LbWithServiceDiscovery> {
    fn n(channel: LbWithServiceDiscovery) -> Self {
        // 使用通道创建新的好友服务客户端
        Self::new(channel)
    }
}

/// 群组服务客户端工厂实现
impl ClientFactory for GroupServiceClient<LbWithServiceDiscovery> {
    fn n(channel: LbWithServiceDiscovery) -> Self {
        // 使用通道创建新的群组服务客户端
        Self::new(channel)
    }
}