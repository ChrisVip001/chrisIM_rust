use std::sync::Arc;

use common::config::AppConfig;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};

pub(crate) use crate::client::Client;
use cache::Cache;
use common::error::Error;
use common::message::chat_service_client::ChatServiceClient;
use common::message::{
    ContentType, GroupMemSeq, Msg, MsgResponse, MsgType, PlatformType, SendMsgRequest,
};
use common::service::chat_client;

/// 用户ID类型别名
type UserID = String;

/// 客户端连接中心类型别名
/// 
/// 使用嵌套的DashMap结构来管理客户端连接：
/// - 外层Map: UserID -> 该用户的所有平台连接
/// - 内层Map: PlatformType -> 特定平台的客户端连接
/// 
/// 这种结构支持：
/// - 同一用户在多个平台同时在线
/// - 快速查找特定用户的连接
/// - 线程安全的并发访问
type Hub = Arc<DashMap<UserID, DashMap<PlatformType, Client>>>;

/// 连接管理器
/// 
/// 负责管理所有WebSocket客户端连接的生命周期，包括：
/// - 客户端注册和注销
/// - 消息路由和分发
/// - 与msg-server的gRPC通信
/// - 缓存操作和序列号管理
/// 
/// ## 核心功能
/// 
/// 1. **连接管理**: 注册/注销客户端，维护在线状态
/// 2. **消息分发**: 根据消息类型路由到正确的接收者
/// 3. **多平台支持**: 同一用户可在多个平台同时在线
/// 4. **序列号管理**: 处理消息发送序列号
/// 5. **错误处理**: 处理网络异常和消息发送失败
#[derive(Clone)]
pub struct Manager {
    /// 消息广播发送器
    /// 用于将客户端发送的消息转发到处理队列
    tx: mpsc::Sender<Msg>,
    
    /// 客户端连接中心
    /// 存储所有在线客户端的连接信息
    pub hub: Hub,
    
    /// 缓存接口
    /// 用于序列号管理和用户状态缓存
    pub cache: Arc<dyn Cache>,
    
    /// 聊天服务RPC客户端
    /// 用于与msg-server通信，发送消息到Kafka队列
    pub chat_rpc: ChatServiceClient<Channel>,
}

#[allow(dead_code)]
impl Manager {
    /// 创建新的连接管理器实例
    /// 
    /// 初始化所有必要的组件：
    /// - 缓存连接
    /// - gRPC客户端
    /// - 连接中心
    /// 
    /// # 参数
    /// * `tx` - 消息广播发送器
    /// * `config` - 应用程序配置
    /// 
    /// # 返回值
    /// 返回配置好的Manager实例
    pub async fn new(tx: mpsc::Sender<Msg>, config: &AppConfig) -> Self {
        // 初始化缓存连接
        let cache = cache::cache(config).await;
        
        // 创建与msg-server的gRPC连接
        let chat_rpc = chat_client()
            .await
            .expect("无法连接到聊天RPC服务");
            
        Manager {
            tx,
            hub: Arc::new(DashMap::new()),
            cache,
            chat_rpc,
        }
    }

    /// 发送群聊消息
    /// 
    /// 处理群聊消息的分发逻辑：
    /// 1. 向发送者的其他平台发送消息副本
    /// 2. 为每个群成员分配正确的序列号
    /// 3. 向所有在线的群成员推送消息
    /// 
    /// # 参数
    /// * `obj_ids` - 群成员序列号信息列表
    /// * `msg` - 要发送的群聊消息
    pub async fn send_group(&self, obj_ids: Vec<GroupMemSeq>, mut msg: Msg) {
        // 向发送者的其他平台发送消息副本
        self.send_to_self(&msg.send_id, &msg).await;

        // 将发送序列号设置为0，因为这是推送给接收者的消息
        msg.send_seq = 0;

        // 为每个群成员发送消息
        for mem in obj_ids {
            if let Some(clients) = self.hub.get(&mem.mem_id) {
                // 为每个成员设置正确的接收序列号
                msg.seq = mem.cur_seq;

                // 向该成员的所有在线客户端发送消息
                self.send_msg_to_clients(&clients, &msg).await;
            }
        }
    }

    /// 向发送者的其他平台发送消息副本
    /// 
    /// 当用户在多个平台同时在线时，需要向发送者的其他平台
    /// 发送消息副本，以保持消息同步。
    /// 
    /// # 参数
    /// * `id` - 发送者用户ID
    /// * `msg` - 要发送的消息
    async fn send_to_self(&self, id: &str, msg: &Msg) {
        if let Some(client) = self.hub.get(id) {
            // 向发送者的另一个平台客户端发送消息
            // 如果当前是移动端发送，则向桌面端发送；反之亦然
            let platform = if msg.platform == PlatformType::Mobile as i32 {
                PlatformType::Desktop
            } else {
                PlatformType::Mobile
            };
            
            if let Some(sender) = client.get(&platform) {
                // 序列化消息
                let content = match bincode::serialize(msg) {
                    Ok(res) => res,
                    Err(_) => {
                        error!("消息序列化失败");
                        return;
                    }
                };
                
                // 发送到另一个平台
                if let Err(e) = sender.send_binary(content).await {
                    error!("向发送者其他平台发送消息失败: {}", e)
                }
            }
        }
    }

    /// 发送单聊消息
    /// 
    /// 处理单聊消息的分发逻辑：
    /// 1. 向接收者的所有在线客户端发送消息
    /// 2. 向发送者的其他平台发送消息副本
    /// 
    /// # 参数
    /// * `obj_id` - 接收者用户ID
    /// * `msg` - 要发送的消息
    pub async fn send_single_msg(&self, obj_id: &str, msg: &Msg) {
        // 向接收者发送消息
        if let Some(clients) = self.hub.get(obj_id) {
            self.send_msg_to_clients(&clients, msg).await;
        }
        
        // 向发送者的其他平台发送消息副本
        self.send_to_self(&msg.send_id, msg).await;
    }

    /// 向客户端连接发送消息
    /// 
    /// 根据客户端数量采用不同的发送策略：
    /// - 0个客户端: 记录错误日志
    /// - 1个客户端: 直接发送
    /// - 2个客户端: 分别发送（支持双端同时在线）
    /// - 超过2个: 记录警告（异常情况）
    /// 
    /// # 参数
    /// * `clients` - 目标用户的所有客户端连接
    /// * `msg` - 要发送的消息
    async fn send_msg_to_clients(&self, clients: &DashMap<PlatformType, Client>, msg: &Msg) {
        match clients.len() {
            0 => error!("未找到客户端连接"),
            1 => {
                // 单个客户端在线
                let content = match bincode::serialize(msg) {
                    Ok(res) => res,
                    Err(e) => {
                        error!("消息序列化失败: {}", e);
                        return;
                    }
                };
                if let Some(client) = clients.iter().next() {
                    if let Err(e) = client.value().send_binary(content).await {
                        error!("发送消息失败: {}", e);
                    }
                }
            }
            2 => {
                // 两个客户端在线（桌面端+移动端）
                let content = match bincode::serialize(msg) {
                    Ok(res) => res,
                    Err(e) => {
                        error!("消息序列化失败: {}", e);
                        return;
                    }
                };
                let mut iter = clients.iter();
                
                // 向第一个客户端发送
                if let Some(first_client) = iter.next() {
                    if let Err(e) = first_client.value().send_binary(content.clone()).await {
                        error!("发送消息失败: {}", e);
                    }
                }
                
                // 向第二个客户端发送
                if let Some(second_client) = iter.next() {
                    if let Err(e) = second_client.value().send_binary(content).await {
                        error!("发送消息失败: {}", e);
                    }
                }
            }
            _ => warn!("客户端数量异常: {}", clients.len()),
        }
    }

    /// 注册客户端连接
    /// 
    /// 将新的客户端连接添加到连接中心。
    /// 支持同一用户在多个平台同时在线。
    /// 
    /// # 参数
    /// * `id` - 用户ID
    /// * `client` - 客户端连接对象
    pub async fn register(&mut self, id: String, client: Client) {
        self.hub
            .entry(id)
            .or_default()
            .insert(client.platform, client);
    }

    /// 注销客户端连接
    /// 
    /// 从连接中心移除指定的客户端连接。
    /// 如果用户的所有平台都断开连接，则完全移除该用户。
    /// 
    /// # 参数
    /// * `id` - 用户ID
    /// * `platform` - 要注销的平台类型
    pub async fn unregister(&mut self, id: String, platform: PlatformType) {
        let mut flag = false;
        if let Some(clients) = self.hub.get_mut(&id) {
            if clients.len() == 1 {
                // 如果这是用户的最后一个连接，标记为需要完全移除
                flag = true;
            } else {
                // 否则只移除特定平台的连接
                clients.remove(&platform);
            }
        };
        
        if flag {
            // 移除用户的所有连接记录
            self.hub.remove(&id);
        }
        debug!("注销客户端连接: {:?}", id);
    }

    /// 运行消息处理循环
    /// 
    /// 这是Manager的主要工作方法，持续监听消息队列：
    /// 1. 接收来自WebSocket客户端的消息
    /// 2. 处理消息（增加序列号、发送到msg-server）
    /// 3. 向发送者返回处理结果
    /// 
    /// # 参数
    /// * `receiver` - 消息接收器
    pub async fn run(&mut self, mut receiver: mpsc::Receiver<Msg>) {
        info!("连接管理器已启动");

        // 从通道读取消息并处理
        while let Some(mut message) = receiver.recv().await {
            self.process_message(&mut message).await;

            // 向发送者回复处理结果
            debug!("回复消息处理结果:{:?}", message);
            self.send_single_msg(&message.send_id, &message).await;
        }
    }

    /// 处理单条消息
    /// 
    /// 执行消息处理的核心逻辑：
    /// 1. 在缓存中增加发送序列号
    /// 2. 通过gRPC将消息发送到msg-server
    /// 3. 处理响应结果和错误情况
    /// 
    /// # 参数
    /// * `message` - 要处理的消息（可变引用，会被修改）
    async fn process_message(&mut self, message: &mut Msg) {
        // 在缓存中增加发送序列号
        // 我们不在这里操作数据库保存发送序列号
        // 这个操作在消费者模块中完成
        // 即使增加失败也不是问题
        match self.cache.incr_send_seq(&message.send_id).await {
            Ok((seq, _, _)) => message.send_seq = seq,
            Err(e) => {
                self.create_error_message(message, e);
                return;
            }
        }

        // 通过gRPC发送消息
        match self.send_rpc_message(message.clone()).await {
            Ok(response) => {
                if response.err.is_empty() {
                    debug!("消息发送成功");
                    // 清空消息内容，避免重复发送
                    message.content.clear();
                } else {
                    error!("消息发送失败: {:?}", response.err);
                    self.create_error_message(message, response.err)
                }
                // 设置响应消息类型和相关信息
                message.msg_type = MsgType::MsgRecResp as i32;
                message.server_id.clone_from(&response.server_id);
                message.send_time = response.send_time;
            }
            Err(err) => {
                error!("gRPC调用失败: {:?}", err);
                self.create_error_message(message, err);
            }
        }
    }

    /// 通过gRPC发送消息到msg-server
    /// 
    /// 将消息包装为gRPC请求并发送到msg-server的ChatService。
    /// 
    /// # 参数
    /// * `message` - 要发送的消息
    /// 
    /// # 返回值
    /// * `Ok(MsgResponse)` - 服务器响应
    /// * `Err(tonic::Status)` - gRPC调用失败
    async fn send_rpc_message(&self, message: Msg) -> Result<MsgResponse, tonic::Status> {
        let mut chat_rpc = self.chat_rpc.clone();
        chat_rpc
            .send_msg(SendMsgRequest {
                message: Some(message),
            })
            .await
            .map(|res| res.into_inner())
    }

    /// 创建错误消息
    /// 
    /// 当消息处理失败时，将错误信息封装为错误消息返回给客户端。
    /// 
    /// # 参数
    /// * `message` - 要修改的消息对象
    /// * `error` - 错误信息
    fn create_error_message(&self, message: &mut Msg, error: impl ToString) {
        message.content_type = ContentType::Error as i32;
        message.msg_type = MsgType::MsgRecResp as i32;
        message.content = error.to_string().into_bytes();
    }

    /// 广播消息
    /// 
    /// 将消息发送到处理队列，由run方法的循环处理。
    /// 
    /// # 参数
    /// * `msg` - 要广播的消息
    /// 
    /// # 返回值
    /// * `Ok(())` - 发送成功
    /// * `Err(Error)` - 发送失败
    pub async fn broadcast(&self, msg: Msg) -> Result<(), Error> {
        self.tx
            .send(msg)
            .await
            .map_err(|e| Error::BroadCastError(e.to_string()))
    }
}
