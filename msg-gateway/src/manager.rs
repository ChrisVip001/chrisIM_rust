use std::sync::Arc;

use common::config::AppConfig;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use base64::{Engine as _, engine::general_purpose};
use axum::extract::ws::Message;

pub(crate) use crate::client::Client;
use cache::Cache;
use common::error::Error;
use common::proto::message::chat_service_client::ChatServiceClient;
use common::proto::message::{
    ContentType, GroupMemSeq, Msg, MsgType, PlatformType, SendMsgRequest
};
use common::service_discovery::LbWithServiceDiscovery;

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
    pub chat_rpc: ChatServiceClient<LbWithServiceDiscovery>,
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
        let chat_rpc = common::grpc_client::base::get_rpc_client(config, config.rpc.chat.name.clone())
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

    /// 向发送者的其他平台发送消息
    /// 
    /// 当用户在多个平台同时在线时，需要向发送者的其他平台
    /// 发送消息副本，以保持消息同步。
    /// 
    /// # 参数
    /// * `id` - 发送者用户ID
    /// * `msg` - 要发送的消息
    async fn send_to_self(&self, id: &str, msg: &Msg) {
        if let Some(clients) = self.hub.get(id) {
            // 创建适合JSON序列化的消息副本
            let mut json_msg = self.prepare_message_for_json(msg);
            
            // 设置is_self标识为true，因为这是发送给发送者自己的消息
            if let Some(obj) = json_msg.as_object_mut() {
                if let Some(conversations) = obj.get_mut("conversations") {
                    if let Some(conversation) = conversations.get_mut(0) {
                        if let Some(conv_obj) = conversation.as_object_mut() {
                            if let Some(messages) = conv_obj.get_mut("recent_messages") {
                                if let Some(message) = messages.get_mut(0) {
                                    if let Some(msg_obj) = message.as_object_mut() {
                                        msg_obj.insert("is_self".to_string(), serde_json::Value::Bool(true));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 序列化消息为JSON
            let content = match serde_json::to_string(&json_msg) {
                Ok(res) => res,
                Err(e) => {
                    error!("消息JSON序列化失败: {}", e);
                    return;
                }
            };

            // 向发送者的所有其他平台发送消息副本
            let mut sent_count = 0;
            for platform_entry in clients.iter() {
                let platform_type = platform_entry.key();
                let client = platform_entry.value();
                // 向其他平台发送消息副本
                if let Err(e) = client.send_text(content.clone()).await {
                    error!("向发送者平台 {:?} 发送消息副本失败: {}", platform_type, e);
                } else {
                    sent_count += 1;
                    debug!("已向发送者平台 {:?} 发送消息副本", platform_type);
                }
            }

            if sent_count > 0 {
                debug!("成功向发送者的 {} 个其他平台发送消息副本", sent_count);
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
    /// # 参数
    /// * `clients` - 目标用户的所有客户端连接
    /// * `msg` - 要发送的消息
    async fn send_msg_to_clients(&self, clients: &DashMap<PlatformType, Client>, msg: &Msg) {
        match clients.len() {
            0 => {
                error!("未找到客户端连接");
                return;
            }
            _ => {}
        }

        // 创建适合JSON序列化的消息副本
        let mut json_msg = self.prepare_message_for_json(msg);
        
        // 设置is_self标识为false，因为这是发送给接收者的消息
        if let Some(obj) = json_msg.as_object_mut() {
            if let Some(conversations) = obj.get_mut("conversations") {
                if let Some(conversation) = conversations.get_mut(0) {
                    if let Some(conv_obj) = conversation.as_object_mut() {
                        if let Some(messages) = conv_obj.get_mut("recent_messages") {
                            if let Some(message) = messages.get_mut(0) {
                                if let Some(msg_obj) = message.as_object_mut() {
                                    msg_obj.insert("is_self".to_string(), serde_json::Value::Bool(false));
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // 序列化为JSON
        let content = match serde_json::to_string(&json_msg) {
            Ok(res) => res,
            Err(e) => {
                error!("消息JSON序列化失败: {}", e);
                return;
            }
        };

        // 向所有在线客户端发送消息
        let mut sent_count = 0;
        for client_entry in clients.iter() {
            let client = client_entry.value();
            if let Err(e) = client.send_text(content.clone()).await {
                error!("发送消息失败: {}", e);
            } else {
                sent_count += 1;
                debug!("已向客户端发送消息");
            }
        }

        if sent_count == 0 {
            error!("未成功发送消息给任何客户端");
        } else {
            debug!("成功向 {} 个客户端发送消息", sent_count);
        }
    }

    /// 为JSON序列化准备消息
    /// 
    /// 根据消息类型将content字段转换为适合的格式：
    /// - SingleMsg/GroupMsg: 将Vec<u8>转换为字符串
    /// - 其他类型: 使用base64编码
    /// 
    /// # 参数
    /// * `msg` - 原始消息
    /// 
    /// # 返回值
    /// 返回适合JSON序列化的消息对象
    fn prepare_message_for_json(&self, msg: &Msg) -> serde_json::Value {
        // 创建一个Conversation对象
        let conversation = common::proto::message::Conversation {
            conversation_id: if msg.msg_type == MsgType::GroupMsg as i32 {
                msg.group_id.clone()
            } else {
                if msg.send_id == msg.receiver_id {
                    msg.receiver_id.clone()
                } else {
                    msg.receiver_id.clone()
                }
            },
            conversation_type: if msg.msg_type == MsgType::GroupMsg as i32 {
                "group".to_string()
            } else {
                "single".to_string()
            },
            recent_messages: vec![msg.clone()],
            unread_count: if !msg.is_read && msg.receiver_id != msg.send_id { 1 } else { 0 },
            last_active_time: msg.send_time,
        };

        // 创建GetConversationsResponse对象
        let response = common::proto::message::GetConversationsResponse {
            conversations: vec![conversation],
            total: 1,
            seq_max: msg.seq,
            send_seq_max: msg.send_seq,
        };

        // 转换为JSON
        serde_json::to_value(response).unwrap_or_default()
    }

    /// 注册客户端连接
    /// 
    /// 将新的客户端连接添加到连接中心。
    /// 支持同一用户在多个平台同时在线。
    /// 
    /// 如果同一平台已存在连接，会主动踢下线旧连接，提升用户体验。
    /// 
    /// # 参数
    /// * `id` - 用户ID
    /// * `client` - 客户端连接对象
    pub async fn register(&mut self, id: String, client: Client) {
        let platforms = self.hub.entry(id.clone()).or_default();
        
        // 检查是否已存在同平台连接
        if let Some(old_client) = platforms.get(&client.platform) {
            info!(
                "检测到用户 {} 在平台 {:?} 的重复连接，准备踢下线旧连接", 
                id, client.platform
            );
            
            // 发送踢下线信号给旧连接
            if let Err(e) = old_client.notify_sender.send(()).await {
                warn!("向用户 {} 平台 {:?} 的旧连接发送踢下线信号失败: {}", id, client.platform, e);
            } else {
                info!("已向用户 {} 平台 {:?} 的旧连接发送踢下线信号", id, client.platform);
            }
            
            // 给旧连接一点时间来处理踢下线信号
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        
        // 保存平台信息用于日志输出
        let platform = client.platform;
        
        // 注册新连接（会覆盖旧连接）
        platforms.insert(platform, client);
        
        info!(
            "用户 {} 在平台 {:?} 的新连接已注册成功", 
            id, platform
        );
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
            // 处理消息并发送到Kafka
            self.process_message(&mut message).await;
            debug!("消息已处理并发送到Kafka: {:?}", message);
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
        // 通过gRPC发送消息
        match self.send_rpc_message(message.clone()).await {
            Ok(response) => {
                debug!("消息发送成功");
                // 完整复制response的所有数据给message
                *message = response;
                // 清空消息内容，避免重复发送
                message.content.clear();
                // 设置响应消息类型
                message.msg_type = MsgType::MsgRecResp as i32;
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
    async fn send_rpc_message(&self, message: Msg) -> Result<Msg, tonic::Status> {
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
