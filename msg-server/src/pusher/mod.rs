use std::{fmt::Debug, sync::Arc};

use common::{
    config::AppConfig,
    error::Error,
    message::{GroupMemSeq, Msg},
};
use tonic::async_trait;

mod service;

/// 消息推送器trait
/// 
/// 定义了消息推送服务的核心接口，负责将消息推送到在线用户。
/// 支持单聊消息和群聊消息的推送，通过WebSocket连接实现实时通信。
/// 
/// 实现类需要处理：
/// - 用户在线状态检查
/// - 消息路由和分发
/// - 连接管理和负载均衡
/// - 推送失败的重试机制
#[async_trait]
pub trait Pusher: Send + Sync + Debug {
    /// 推送单聊消息
    /// 
    /// 将消息推送给指定的单个用户，如果用户在线则立即推送，
    /// 如果用户离线则消息会保存在离线消息盒子中。
    /// 
    /// # 参数
    /// * `msg` - 要推送的消息对象
    /// 
    /// # 返回值
    /// * `Ok(())` - 推送成功或用户离线
    /// * `Err(Error)` - 推送失败的错误信息
    async fn push_single_msg(&self, msg: Msg) -> Result<(), Error>;
    
    /// 推送群聊消息
    /// 
    /// 将消息推送给群组中的所有在线成员，每个成员都会收到
    /// 带有自己序列号的消息副本。离线成员的消息会保存在各自的消息盒子中。
    /// 
    /// # 参数
    /// * `msg` - 要推送的群聊消息
    /// * `members` - 群成员列表，包含每个成员的序列号信息
    /// 
    /// # 返回值
    /// * `Ok(())` - 推送成功
    /// * `Err(Error)` - 推送失败的错误信息
    async fn push_group_msg(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error>;
}

/// 创建消息推送服务实例
/// 
/// 根据配置创建合适的推送服务实现，目前支持基于WebSocket的推送。
/// 推送服务会自动处理服务发现、负载均衡等功能。
/// 
/// # 参数
/// * `config` - 应用程序配置，包含WebSocket服务的连接信息
/// 
/// # 返回值
/// * `Ok(Arc<dyn Pusher>)` - 推送服务实例
/// * `Err(Error)` - 创建失败的错误信息
pub async fn push_service(config: &AppConfig) -> Result<Arc<dyn Pusher>, Error> {
    let service = service::PusherService::new(config).await?;
    Ok(Arc::new(service))
}
