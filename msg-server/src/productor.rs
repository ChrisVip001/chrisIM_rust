use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nanoid::nanoid;
use rdkafka::producer::{FutureProducer, FutureRecord};
use tonic::transport::Server;
use tracing::{debug, error, info, warn};

use cache::{ConversationIndexCache, RelationCache, UnreadCache};
use runtime::grpc_client::base::get_rpc_client;
use runtime::grpc_client::base::service_names as sn;
use contract::proto::friend::friend_service_client::FriendServiceClient;
use contract::proto::group::group_service_client::GroupServiceClient;
use contract::proto::message::chat_service_server::{ChatService, ChatServiceServer};
use contract::proto::message::{
    DeleteMessagesRequest, DeleteMessagesResponse, ForwardMessageRequest, ForwardMessageResponse,
    GetConversationsRequest, GetConversationsResponse, GetDbMessagesRequest,
    GetMessageHistoryResponse, MarkConversationAsReadRequest, MarkMessagesAsReadRequest,
    MarkMessagesAsReadResponse, Msg, MsgType, RevokeMessageRequest, RevokeMessageResponse,
    SendMsgRequest,
};
use runtime::service_discovery::LbWithServiceDiscovery;
use runtime::service_register_center::service_register_center;
use runtime::config::{AppConfig, Component};
use runtime::grpc::LoggingInterceptor;
use contract::Error;
use contract::types::conversation::{conversation_id_of, peer_of_single};
use msg_storage::ReadSpreadRepo;

use crate::kafka;

/// 会话列表一次最多返回的会话数
const CONVERSATION_LIST_LIMIT: i64 = 50;

/// 单个会话首屏/增量一次拉取的消息条数
const CONVERSATION_RECENT_LIMIT: i64 = 20;

/// 读扩散下 `receiver_id` 是用户视角字段：
/// 单聊由会话 ID 反推对端，群聊留空（`group_id` 已由存储层填好）。
fn fill_single_receiver(
    messages: &mut [contract::proto::message::Msg],
    conversation_id: &str,
    user_id: &str,
) {
    if let Some(peer) = peer_of_single(conversation_id, user_id) {
        for msg in messages.iter_mut() {
            if msg.receiver_id.is_empty() {
                msg.receiver_id = peer.clone();
            }
        }
    }
}

/// 聊天消息RPC服务实现
///
/// 这是msg-server的核心服务之一，负责：
/// 1. 通过gRPC接收来自客户端的消息发送请求
/// 2. 为消息生成唯一的服务器ID和时间戳
/// 3. 将消息序列化后发送到Kafka消息队列
/// 4. 向客户端返回发送结果
/// 5. 处理消息查询和已读状态更新
///
/// 工作流程：
/// 客户端 -> gRPC请求 -> ChatRpcService -> Kafka队列 -> ConsumerService
pub struct ChatRpcService {
    /// Kafka生产者实例，用于将消息发送到Kafka消息队列
    /// 配置了重试、幂等性等参数确保消息可靠投递
    kafka: FutureProducer,

    /// Kafka主题名称，所有消息都将发送到这个主题
    /// 消费者服务会从同一个主题消费消息进行后续处理
    topic: String,

    /// 读扩散仓储（会话、消息元数据、已读游标）
    read: Arc<ReadSpreadRepo>,

    /// 关系缓存（好友/黑名单校验），只依赖 RelationCache 窄接口
    relation: Arc<dyn RelationCache>,

    /// 未读计数缓存，只依赖 UnreadCache 窄接口
    unread: Arc<dyn UnreadCache>,

    /// 会话列表索引（按最近活跃时间排序）
    conversations: Arc<dyn ConversationIndexCache>,

    /// 好友服务客户端
    friend_client: FriendServiceClient<LbWithServiceDiscovery>,

    /// 群组服务客户端
    /// 用于查询群组成员信息、处理群组相关操作
    group_client: GroupServiceClient<LbWithServiceDiscovery>,

    /// 校验链路故障时是否放行（fail-open）
    ///
    /// false（默认）：故障即拒绝，安全优先；true：故障放行，可用性优先。
    fail_open: bool,
}

impl ChatRpcService {
    /// 创建一个新的聊天RPC服务实例
    ///
    /// # 参数
    /// * `kafka` - 已配置的Kafka生产者实例
    /// * `topic` - 消息要发送到的Kafka主题名称
    /// * `msg_storage` - 消息存储仓库
    /// * `cache` - 缓存接口
    /// * `friend_client` - 好友服务客户端
    /// * `group_client` - 群组服务客户端
    ///
    /// # 返回值
    /// 返回ChatRpcService实例
    pub fn new(
        kafka: FutureProducer,
        topic: String,
        read: Arc<ReadSpreadRepo>,
        relation: Arc<dyn RelationCache>,
        unread: Arc<dyn UnreadCache>,
        conversations: Arc<dyn ConversationIndexCache>,
        friend_client: FriendServiceClient<LbWithServiceDiscovery>,
        group_client: GroupServiceClient<LbWithServiceDiscovery>,
        fail_open: bool,
    ) -> Self {
        Self {
            kafka,
            topic,
            read,
            relation,
            unread,
            conversations,
            friend_client,
            group_client,
            fail_open,
        }
    }

    /// 启动聊天消息服务
    ///
    /// 这是服务的主要启动方法，执行以下步骤：
    /// 1. 初始化和配置Kafka生产者
    /// 2. 确保Kafka主题存在
    /// 3. 向服务注册中心注册当前服务
    /// 4. 启动gRPC服务器并监听客户端请求
    ///
    /// # 参数
    /// * `config` - 应用程序配置，包含Kafka、gRPC等所有配置信息
    pub async fn start(config: &AppConfig) -> Result<(), Error> {
        // 创建Kafka生产者（参数见 kafka 模块，与消费者侧共用同一套可靠投递配置）
        let producer = kafka::build_producer(config)?;

        // 确保Kafka主题存在（broker/超时/分区/副本由 kafka 模块从配置推导）
        kafka::ensure_topic_for(config, &config.kafka.topic)
        .await
        .map_err(|e| Error::Internal(format!("确保Kafka主题存在失败: {}", e)))?;

        info!("Kafka主题 '{}' 已确保存在", config.kafka.topic);

        // 注册后持有 service_id：停机时先摘除注册流量，
        // 避免 TTL 窗口内仍有新消息被路由到已停实例。
        let service_id =
            runtime::grpc_client::base::register_service(config, Component::MessageServer).await?;

        info!("聊天RPC服务已注册到服务注册中心: {}", service_id);

        // 初始化读扩散仓储与缓存
        // 同一 Redis 实例按各模块的真实依赖收窄为领域 trait
        let read = Arc::new(ReadSpreadRepo::from_config(config).await?);

        let redis = cache::cache(config).await;
        let relation: Arc<dyn RelationCache> = redis.clone();
        let unread: Arc<dyn UnreadCache> = redis.clone();
        let conversations: Arc<dyn ConversationIndexCache> = redis.clone();

        // 初始化好友和群组服务客户端
        // 用户/好友/群组是模块化单体，统一注册在 relation 服务名下
        let friend_client = get_rpc_client::<FriendServiceClient<LbWithServiceDiscovery>>(config, sn::RELATION.to_string())
            .await?;

        let group_client = get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(config, sn::RELATION.to_string())
            .await?;

        // 创建gRPC健康检查服务
        // 用于监控服务健康状态，支持Kubernetes等容器编排工具的健康检查
        let (health_reporter, health_service) = tonic_health::server::health_reporter();

        // 设置服务为健康状态
        // 当服务能正常启动时就认为是健康的
        health_reporter
            .set_serving::<ChatServiceServer<ChatRpcService>>()
            .await;

        // 创建gRPC日志拦截器
        // 用于记录所有RPC请求和响应，便于调试和监控
        let logging_interceptor = LoggingInterceptor::new();

        // 创建聊天RPC服务实例并包装为gRPC服务
        let chat_rpc = Self::new(
            producer,
            config.kafka.topic.clone(),
            read,
            relation,
            unread,
            conversations,
            friend_client,
            group_client,
            config.validation.fail_open,
        );
        let service = ChatServiceServer::with_interceptor(chat_rpc, logging_interceptor);

        info!(
            "聊天RPC服务已启动，监听地址: {}",
            config.rpc.chat.rpc_server_url()
        );

        let addr = config
            .rpc
            .chat
            .rpc_server_url()
            .parse()
            .map_err(|e| Error::Internal(format!("解析服务器地址失败: {}", e)))?;

        // 收到 SIGTERM/SIGINT 后先注销注册（同时停掉 TTL 上报任务），
        // tonic 再停止接收新请求并等待在途请求结束。
        let registry = service_register_center(config);
        let shutdown = async move {
            runtime::shutdown::wait_for_shutdown_signal().await;
            if let Err(e) = registry.deregister(&service_id).await {
                error!("注销 chat 服务失败: {}", e);
            }
        };

        // 启动gRPC服务器
        // 同时提供健康检查服务和聊天消息服务
        Server::builder()
            .add_service(health_service) // 健康检查服务
            .add_service(service) // 聊天消息服务
            .serve_with_shutdown(addr, shutdown)
            .await
            .map_err(|e| Error::Internal(format!("启动gRPC服务器失败: {}", e)))?;

        Ok(())
    }

    /// 组装会话列表响应
    ///
    /// 会话元信息一次批量取回；`backfill` 为真时顺带把索引补上，
    /// 下次请求直接命中 Redis。
    async fn build_conversations(
        &self,
        user_id: &str,
        cursors: std::collections::HashMap<String, i64>,
        states: Vec<msg_storage::read_spread::UserConversationState>,
        backfill: bool,
    ) -> Result<tonic::Response<GetConversationsResponse>, tonic::Status> {
        let ids: Vec<String> = states.iter().map(|s| s.conversation_id.clone()).collect();
        let mut by_id: std::collections::HashMap<String, msg_storage::read_spread::ConversationState> =
            match self.read.conversation.get_many(&ids).await {
                Ok(list) => list
                    .into_iter()
                    .map(|c| (c.conversation_id.clone(), c))
                    .collect(),
                Err(e) => {
                    error!("批量查询会话失败: {}", e);
                    return Err(tonic::Status::internal("获取会话列表失败"));
                }
            };

        if backfill && !states.is_empty() {
            let entries: Vec<(String, String, i64)> = states
                .iter()
                .filter_map(|s| {
                    by_id
                        .get(&s.conversation_id)
                        .map(|c| (user_id.to_string(), s.conversation_id.clone(), c.last_msg_time))
                })
                .collect();
            let index = self.conversations.clone();
            tokio::spawn(async move {
                if let Err(e) = index.touch_conversation_index(&entries).await {
                    warn!("回填会话列表索引失败: {}", e);
                }
            });
        }

        let mut conversations = Vec::with_capacity(states.len());

        for state in states {
            let conv_id = state.conversation_id.clone();

            let Some(conv_state) = by_id.remove(&conv_id) else {
                continue;
            };

            // 增量：从客户端水位之后补；首屏：取最近 N 条
            let since = cursors.get(conv_id.as_str()).copied().unwrap_or(0);
            let msgs = if since > 0 {
                self.read
                    .meta
                    .fetch_after(&conv_id, since, CONVERSATION_RECENT_LIMIT)
                    .await
            } else if conv_state.last_msg_seq < 1 {
                Ok(Vec::new())
            } else {
                let start = (conv_state.last_msg_seq - CONVERSATION_RECENT_LIMIT + 1).max(1);
                self.read
                    .meta
                    .fetch_range(&conv_id, start, conv_state.last_msg_seq)
                    .await
            };

            let mut msgs = match msgs {
                Ok(m) => m,
                Err(e) => {
                    error!("拉取会话 {} 消息失败: {}", conv_id, e);
                    continue;
                }
            };

            if msgs.is_empty() {
                continue;
            }

            fill_single_receiver(&mut msgs, &conv_id, user_id);

            // 位于游标之前的消息视为已读
            for msg in msgs.iter_mut() {
                msg.is_read = msg.seq > 0 && msg.seq <= state.last_read_seq;
            }

            let unread = (conv_state.last_msg_seq - state.last_read_seq).max(0) as i32;
            let last_active_time = msgs.last().map(|m| m.send_time).unwrap_or(0);
            let conversation_type = if conv_state.conv_type == 2 {
                "group"
            } else {
                "single"
            };

            if let Err(e) = self
                .unread
                .unread_count_set(user_id, &conv_id, &unread)
                .await
            {
                warn!("写入未读计数缓存失败: {}", e);
            }

            conversations.push(contract::proto::message::Conversation {
                conversation_id: conv_id,
                conversation_type: conversation_type.to_string(),
                recent_messages: msgs,
                unread_count: unread,
                last_active_time,
            });
        }

        // 最近活跃的排在最前，与索引的排序方向一致
        conversations.sort_by_key(|c| std::cmp::Reverse(c.last_active_time));
        let total = conversations.len() as i32;

        Ok(tonic::Response::new(GetConversationsResponse {
            conversations,
            total,
            seq_max: 0,
            send_seq_max: 0,
        }))
    }
}

/// 消息发送权限校验（拆分至独立子模块，关注点：「能不能发」）
mod validation;

/// 实现gRPC的ChatService trait
/// 提供消息发送的具体业务逻辑
#[async_trait]
impl ChatService for ChatRpcService {
    /// 发送消息到Kafka消息队列
    ///
    /// 这是核心的消息处理方法，执行以下步骤：
    /// 1. 验证和处理消息内容
    /// 2. 生成唯一的服务器消息ID
    /// 3. 设置消息发送时间戳
    /// 4. 将消息序列化并发送到Kafka
    /// 5. 返回处理结果给客户端
    ///
    /// # 参数
    /// * `request` - gRPC请求，包含要发送的消息
    ///
    /// # 返回值
    /// * `Ok(Response<MsgResponse>)` - 发送成功，返回消息响应
    /// * `Err(Status)` - 发送失败，返回错误状态
    async fn send_msg(
        &self,
        request: tonic::Request<SendMsgRequest>,
    ) -> Result<tonic::Response<Msg>, tonic::Status> {
        // 从gRPC请求中提取消息内容
        let mut msg = request
            .into_inner()
            .message
            .ok_or(tonic::Status::invalid_argument("消息内容不能为空"))?;

        // 根据消息类型校验发送权限
        self.validate_message_permissions(&msg).await?;
        
        // 使用nanoid生成唯一消息ID, nanoid比UUID更短且更安全
        msg.server_id = nanoid!();

        // 读扩散下不预分配序号：消息的会话级 seq 由消费端按 conversation 统一分配。
        // 发送端预分配会在批量或重试时产生与落库顺序不一致的号。

        // 设置消息的服务器发送时间戳（毫秒级）
        msg.send_time = chrono::Utc::now().timestamp_millis();

        // 将消息对象序列化为JSON字符串
        let payload = serde_json::to_string(&msg).unwrap();

        // 创建Kafka消息记录
        // 分区键必须是会话ID：同一会话落同一分区，
        // 消费端才能按分区有序处理、序号不乱序
        let key = conversation_id_of(&msg);
        let record: FutureRecord<'_, str, String> = FutureRecord::to(&self.topic)
            .payload(&payload)
            .key(&key);

        // 发送消息到Kafka
        match self.kafka.send(record, Duration::from_secs(10)).await {
            Ok(_) => {
                debug!("消息已成功发送到Kafka: {}", msg.server_id);

                // 返回成功响应
                Ok(tonic::Response::new(msg))
            }
            Err((kafka_error, _)) => {
                error!("发送消息到Kafka失败: {}", kafka_error);

                // 返回gRPC错误状态
                Err(tonic::Status::internal(format!(
                    "消息发送失败: {}",
                    kafka_error
                )))
            }
        }
    }

    /// 标记消息已读
    async fn mark_messages_as_read(
        &self,
        request: tonic::Request<MarkMessagesAsReadRequest>,
    ) -> Result<tonic::Response<MarkMessagesAsReadResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!(
            "标记消息已读请求: user_id={}, msg_seqs={:?}",
            req.user_id, req.msg_seqs
        );

        if req.msg_seqs.is_empty() {
            return Err(tonic::Status::invalid_argument("消息序列号列表不能为空"));
        }

        if req.conversation_id.is_empty() {
            return Err(tonic::Status::invalid_argument("会话ID不能为空"));
        }

        // 读扩散：把列表中的最大序号作为会话游标
        let read_seq = req.msg_seqs.iter().copied().max().unwrap_or(0);
        match self
            .read
            .user_conversation
            .mark_read(&req.user_id, &req.conversation_id, read_seq)
            .await
        {
            Ok(()) => {
                debug!(
                    "会话 {} 已读游标推进到 {}",
                    req.conversation_id, read_seq
                );
                Ok(tonic::Response::new(MarkMessagesAsReadResponse {
                    success: true,
                    read_count: req.msg_seqs.len() as i32,
                    error: String::new(),
                }))
            }
            Err(e) => {
                error!("标记消息已读失败: {}", e);
                Err(tonic::Status::internal(format!("标记消息已读失败: {}", e)))
            }
        }
    }

    /// 标记会话已读
    async fn mark_conversation_as_read(
        &self,
        request: tonic::Request<MarkConversationAsReadRequest>,
    ) -> Result<tonic::Response<MarkMessagesAsReadResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!(
            "标记会话已读请求: user_id={}, conversation_id={}, up_to_time={:?}",
            req.user_id, req.conversation_id, req.up_to_time
        );

        // 验证参数
        if req.user_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID不能为空"));
        }

        if req.conversation_id.is_empty() {
            return Err(tonic::Status::invalid_argument("会话ID不能为空"));
        }

        // 读扩散：已读是会话级游标，只写 1 行，与消息条数、群成员数无关
        let before = self
            .read
            .user_conversation
            .unread(&req.user_id, &req.conversation_id)
            .await
            .unwrap_or(0);

        let read_seq = match self.read.conversation.get(&req.conversation_id).await {
            Ok(Some(state)) => state.last_msg_seq,
            Ok(None) => 0,
            Err(e) => {
                error!("查询会话水位失败: {}", e);
                return Err(tonic::Status::internal(format!("查询会话失败: {}", e)));
            }
        };

        if let Err(e) = self
            .read
            .user_conversation
            .mark_read(&req.user_id, &req.conversation_id, read_seq)
            .await
        {
            error!("标记会话已读失败: {}", e);
            return Err(tonic::Status::internal(format!("标记会话已读失败: {}", e)));
        }

        let after = self
            .read
            .user_conversation
            .unread(&req.user_id, &req.conversation_id)
            .await
            .unwrap_or(0);
        let count = (before - after).max(0) as i32;

        if let Err(e) = self
            .unread
            .unread_count_remove(&req.user_id, &req.conversation_id)
            .await
        {
            error!("清除未读计数失败: {}", e);
        }

        debug!(
            "会话 {} 已读推进到 {}，清除未读 {} 条",
            req.conversation_id, read_seq, count
        );
        Ok(tonic::Response::new(MarkMessagesAsReadResponse {
            success: true,
            read_count: count,
            error: String::new(),
        }))
    }

    /// 根据用户ID、会话ID和序列号范围获取消息历史
    ///
    /// 逻辑说明：
    /// - send_seq_start/send_seq_end: 用户发送消息的序列号范围
    /// - seq_start/seq_end: 用户接收消息的序列号范围
    /// - conversation_id: 会话ID，用于过滤特定会话的消息
    /// - 如果序列号范围为0，则使用当前序列号作为参考
    async fn get_message_history(
        &self,
        request: tonic::Request<GetDbMessagesRequest>,
    ) -> Result<tonic::Response<GetMessageHistoryResponse>, tonic::Status> {
        let req = request.into_inner();

        debug!("获取消息历史请求: user_id={}, conversation_id={}, send_seq_start={}, send_seq_end={}, seq_start={}, seq_end={}", 
               req.user_id, req.conversation_id, req.send_seq_start, req.send_seq_end, req.seq_start, req.seq_end);

        // 验证必需参数
        GetDbMessagesRequest::validate(&req)
            .map_err(|e| tonic::Status::invalid_argument(e.to_string()))?;

        // 读扩散：以会话级序号区间定位，user 级双水位不再参与查询
        let end_seq = if req.seq_end > 0 {
            req.seq_end
        } else {
            match self.read.conversation.get(&req.conversation_id).await {
                Ok(Some(state)) => state.last_msg_seq,
                Ok(None) => 0,
                Err(e) => {
                    error!("查询会话水位失败: {}", e);
                    return Err(tonic::Status::internal(format!("查询会话失败: {}", e)));
                }
            }
        };

        let start_seq = req.seq_start.max(1);
        debug!("会话 {} 查询区间: {}-{}", req.conversation_id, start_seq, end_seq);

        if end_seq < start_seq {
            return Ok(tonic::Response::new(GetMessageHistoryResponse {
                messages: Vec::new(),
                conversation_id: req.conversation_id,
            }));
        }

        match self
            .read
            .meta
            .fetch_range(&req.conversation_id, start_seq, end_seq)
            .await
        {
            Ok(mut messages) => {
                fill_single_receiver(&mut messages, &req.conversation_id, &req.user_id);
                debug!("成功获取 {} 条消息", messages.len());
                Ok(tonic::Response::new(GetMessageHistoryResponse {
                    messages,
                    conversation_id: req.conversation_id,
                }))
            }
            Err(e) => {
                error!("获取消息历史失败: {}", e);
                Err(tonic::Status::internal(format!("获取消息历史失败: {}", e)))
            }
        }
    }

    /// 获取会话列表
    async fn get_conversations(
        &self,
        request: tonic::Request<GetConversationsRequest>,
    ) -> Result<tonic::Response<GetConversationsResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!(
            "获取会话列表请求，用户ID: {}, 同步模式: {:?}",
            req.user_id, req.sync_mode
        );

        // 方案 A：客户端按会话上报水位，服务端据此补齐增量
        let cursors: std::collections::HashMap<String, i64> = req
            .cursors
            .iter()
            .map(|c| (c.conversation_id.clone(), c.since_seq))
            .collect();

        // 会话顺序来自 Redis 索引（按最近活跃时间倒序）；
        // 索引缺失时退回数据库的排序查询，并在后台把索引补回来
        let (states, from_index) = match self
            .conversations
            .recent_conversations(&req.user_id, CONVERSATION_LIST_LIMIT)
            .await
        {
            Ok(indexed) if !indexed.is_empty() => {
                let order: std::collections::HashMap<String, usize> = indexed
                    .into_iter()
                    .enumerate()
                    .map(|(position, (conversation_id, _))| (conversation_id, position))
                    .collect();
                let ids: Vec<String> = order.keys().cloned().collect();

                let mut states = match self
                    .read
                    .user_conversation
                    .list_by_ids(&req.user_id, &ids)
                    .await
                {
                    Ok(states) => states,
                    Err(e) => {
                        error!("获取用户会话状态失败: {}", e);
                        return Err(tonic::Status::internal("获取会话列表失败"));
                    }
                };
                // 索引决定展示顺序，数据库只负责补齐游标
                states.sort_by_key(|s| {
                    order
                        .get(&s.conversation_id)
                        .copied()
                        .unwrap_or(usize::MAX)
                });
                (states, true)
            }
            _ => {
                let states = match self
                    .read
                    .user_conversation
                    .list(&req.user_id, CONVERSATION_LIST_LIMIT)
                    .await
                {
                    Ok(states) => states,
                    Err(e) => {
                        error!("获取用户会话列表失败: {}", e);
                        return Err(tonic::Status::internal("获取会话列表失败"));
                    }
                };
                (states, false)
            }
        };

        self.build_conversations(&req.user_id, cursors, states, !from_index)
            .await
    }

    /// 撤回消息
    async fn revoke_message(
        &self,
        request: tonic::Request<RevokeMessageRequest>,
    ) -> Result<tonic::Response<RevokeMessageResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!(
            "撤回消息请求: user_id={}, message_id={}",
            req.user_id, req.message_id
        );

        // 验证参数
        if req.user_id.is_empty() || req.message_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID和消息ID不能为空"));
        }

        if req.conversation_id.is_empty() {
            return Err(tonic::Status::invalid_argument("会话ID不能为空"));
        }

        // 首先检查消息是否存在以及撤回时间限制
        match self
            .read
            .meta
            .get(&req.conversation_id, &req.message_id)
            .await
        {
            Ok(Some(msg)) => {
                // 验证消息是否属于当前用户
                if msg.send_id != req.user_id {
                    return Err(tonic::Status::internal("只能撤回自己发送的消息"));
                }

                // 验证消息是否已经被撤回
                if msg.is_revoked {
                    return Err(tonic::Status::not_found("消息已经被撤回"));
                }

                // 验证撤回时间限制（2分钟内）
                let now = chrono::Utc::now().timestamp_millis();
                let time_limit = 2 * 60 * 1000; // 2分钟
                if now - msg.send_time > time_limit {
                    return Err(tonic::Status::internal("消息发送超过2分钟，无法撤回"));
                }

                // 执行撤回操作
                match self
                    .read
                    .meta
                    .revoke(&req.conversation_id, msg.seq, &req.user_id, now)
                    .await
                {
                    Ok(()) => {
                        debug!("消息撤回成功: {}", req.message_id);
                        
                        // 发送一条撤回通知消息到Kafka，让其他用户知道消息被撤回了
                        let mut revoke_notification = msg.clone(); // 直接克隆原消息
                        // 更新撤回相关字段
                        revoke_notification.is_revoked = true;
                        revoke_notification.revoke_time = now;
                        revoke_notification.revoked_by = req.user_id.clone();

                        // 将撤回后的消息发送到Kafka
                        let revoke_payload = serde_json::to_string(&revoke_notification).unwrap();
                        let revoke_key = conversation_id_of(&revoke_notification);
                        let revoke_record: FutureRecord<'_, str, String> =
                            FutureRecord::to(&self.topic)
                                .payload(&revoke_payload)
                                .key(&revoke_key);

                        match self.kafka.send(revoke_record, Duration::from_secs(10)).await {
                            Ok(_) => {
                                debug!("撤回消息状态已发送到Kafka: {}", revoke_notification.server_id);
                            }
                            Err((kafka_error, _)) => {
                                error!("发送撤回消息状态到Kafka失败: {}", kafka_error);
                                // 即使通知发送失败，撤回操作也已经成功，不应该返回错误
                            }
                        }

                        Ok(tonic::Response::new(RevokeMessageResponse {
                            success: true,
                            error: String::new(),
                            revoke_time: now,
                        }))
                    }
                    Err(e) => {
                        error!("撤回消息失败: {}", e);
                        Err(tonic::Status::internal(format!("撤回消息失败: {}", e)))
                    }
                }
            }
            Ok(None) => Err(tonic::Status::not_found("消息不存在")),
            Err(e) => {
                error!("查询消息失败: {}", e);
                Err(tonic::Status::internal(format!("查询消息失败: {}", e)))
            }
        }
    }

    /// 删除消息
    async fn delete_messages(
        &self,
        request: tonic::Request<DeleteMessagesRequest>,
    ) -> Result<tonic::Response<DeleteMessagesResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!(
            "删除消息请求: user_id={}, message_ids={:?}, message_seqs={:?}",
            req.user_id, req.message_ids, req.message_seqs
        );

        // 验证参数
        if req.user_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID不能为空"));
        }

        if req.message_ids.is_empty() && req.message_seqs.is_empty() {
            return Err(tonic::Status::invalid_argument(
                "必须提供消息ID或消息序列号",
            ));
        }

        if req.conversation_id.is_empty() {
            return Err(tonic::Status::invalid_argument("会话ID不能为空"));
        }

        let mut total_deleted = 0i32;

        // 按消息ID删除
        if !req.message_ids.is_empty() {
            match self
                .read
                .meta
                .delete_by_server_id(&req.conversation_id, &req.message_ids)
                .await
            {
                Ok(count) => {
                    total_deleted += count as i32;
                    debug!("按消息ID删除了 {} 条消息", count);
                }
                Err(e) => {
                    error!("按消息ID删除消息失败: {}", e);
                    return Err(tonic::Status::internal(format!("删除消息失败: {}", e)));
                }
            }
        }

        // 按序号删除：读扩散下先按会话定位出 server_id，再批量删除
        if !req.message_seqs.is_empty() {
            let mut ids = Vec::with_capacity(req.message_seqs.len());
            for seq in &req.message_seqs {
                match self
                    .read
                    .meta
                    .fetch_range(&req.conversation_id, *seq, *seq)
                    .await
                {
                    Ok(mut msgs) => {
                        if let Some(m) = msgs.pop() {
                            ids.push(m.server_id);
                        }
                    }
                    Err(e) => {
                        error!("按序号定位消息失败: {}", e);
                        return Err(tonic::Status::internal(format!("删除消息失败: {}", e)));
                    }
                }
            }

            if !ids.is_empty() {
                match self
                    .read
                    .meta
                    .delete_by_server_id(&req.conversation_id, &ids)
                    .await
                {
                    Ok(count) => {
                        total_deleted += count as i32;
                        debug!("按序号删除了 {} 条消息", count);
                    }
                    Err(e) => {
                        error!("按序号删除消息失败: {}", e);
                        return Err(tonic::Status::internal(format!("删除消息失败: {}", e)));
                    }
                }
            }
        }

        Ok(tonic::Response::new(DeleteMessagesResponse {
            success: true,
            deleted_count: total_deleted,
            error: String::new(),
        }))
    }

    /// 转发消息
    async fn forward_message(
        &self,
        request: tonic::Request<ForwardMessageRequest>,
    ) -> Result<tonic::Response<ForwardMessageResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!(
            "转发消息请求: user_id={}, message_ids={:?}, targets={:?}/{:?}",
            req.user_id, req.message_ids, req.target_user_ids, req.target_group_ids
        );

        // 验证参数
        if req.user_id.is_empty() || req.message_ids.is_empty() {
            return Err(tonic::Status::invalid_argument(
                "用户ID和原始消息ID不能为空",
            ));
        }

        if req.target_user_ids.is_empty() && req.target_group_ids.is_empty() {
            return Err(tonic::Status::invalid_argument("必须指定转发目标"));
        }

        if req.source_conversation_id.is_empty() {
            return Err(tonic::Status::invalid_argument("原始消息所属会话不能为空"));
        }

        // 读扩散：按会话 + server_id 逐条取回原始消息
        let mut original_messages = Vec::with_capacity(req.message_ids.len());
        for id in &req.message_ids {
            match self
                .read
                .meta
                .get(&req.source_conversation_id, id)
                .await
            {
                Ok(Some(msg)) => original_messages.push(msg),
                Ok(None) => {}
                Err(e) => {
                    return Err(tonic::Status::internal(format!("获取原始消息失败: {}", e)))
                }
            }
        }
        debug!("获取原始消息成功: {} 条", original_messages.len());

        if original_messages.is_empty() {
            return Err(tonic::Status::not_found("未找到要转发的消息"));
        }

        // 检查原始消息是否允许转发
        for original_msg in &original_messages {
            if !self.is_message_forwardable(original_msg).await? {
                return Err(tonic::Status::permission_denied(
                    format!("消息 {} 不允许转发", original_msg.server_id)
                ));
            }
        }

        let mut forwarded_message_ids = Vec::new();
        let mut success_count = 0;
        let mut error_messages = Vec::new();

        if !req.target_user_ids.is_empty() {
            // 转发给单聊用户
            for target_user_id in &req.target_user_ids {
                // 检查转发到单聊的权限
                if self
                    .check_forward_to_user_permission(&req.user_id, target_user_id)
                    .await
                    .is_err()
                {
                    error_messages.push(format!("无法转发给用户 {}", target_user_id));
                    continue;
                }

                // 为每条原始消息创建转发消息
                for original_msg in &original_messages {
                    let forward_msg = Msg {
                        send_id: req.user_id.clone(),
                        receiver_id: target_user_id.clone(),
                        local_id: format!("forward_{}", nanoid::nanoid!()),
                        server_id: nanoid::nanoid!(),
                        create_time: chrono::Utc::now().timestamp_millis(),
                        send_time: chrono::Utc::now().timestamp_millis(),
                        seq: 0,
                        send_seq: 0,
                        msg_type: MsgType::SingleMsg as i32,
                        content_type: original_msg.content_type,
                        content: original_msg.content.clone(),
                        is_read: false,
                        group_id: String::new(),
                        platform: original_msg.platform,
                        avatar: String::new(),
                        nickname: String::new(),
                        related_msg_id: None,
                        is_revoked: false,
                        revoke_time: 0,
                        revoked_by: String::new(),
                    };

                    // 发送转发消息到Kafka
                    let payload = serde_json::to_string(&forward_msg).unwrap();
                    let key = conversation_id_of(&forward_msg);
                    let record: FutureRecord<'_, str, String> = FutureRecord::to(&self.topic)
                        .payload(&payload)
                        .key(&key);

                    match self.kafka.send(record, Duration::from_secs(10)).await {
                        Ok(_) => {
                            forwarded_message_ids.push(forward_msg.server_id.clone());
                            success_count += 1;
                            debug!(
                                "转发消息成功: {} -> {}",
                                original_msg.server_id, forward_msg.server_id
                            );
                        }
                        Err((kafka_error, _)) => {
                            error!("转发消息到Kafka失败: {}", kafka_error);
                            error_messages.push(format!("转发消息到Kafka失败: {}", kafka_error));
                        }
                    }
                }
            }
        }

        if !req.target_group_ids.is_empty() {
            // 转发给群组
            for target_group_id in &req.target_group_ids {
                // 检查转发到群组的权限
                if let Err(e) = self.check_forward_to_group_permission(&req.user_id, target_group_id).await {
                    error_messages.push(format!("无法转发给群组 {}: {}", target_group_id, e));
                    continue;
                }

                // 为每条原始消息创建转发消息
                for original_msg in &original_messages {
                    let forward_msg = Msg {
                        send_id: req.user_id.clone(),
                        receiver_id: target_group_id.clone(),
                        local_id: String::new(),
                        server_id: nanoid::nanoid!(),
                        create_time: chrono::Utc::now().timestamp_millis(),
                        send_time: chrono::Utc::now().timestamp_millis(),
                        seq: 0,
                        send_seq: 0,
                        msg_type: MsgType::GroupMsg as i32,
                        content_type: original_msg.content_type,
                        content: original_msg.content.clone(),
                        is_read: false,
                        group_id: target_group_id.clone(),
                        platform: original_msg.platform,
                        avatar: String::new(),
                        nickname: String::new(),
                        related_msg_id: None,
                        is_revoked: false,
                        revoke_time: 0,
                        revoked_by: String::new(),
                    };

                    // 发送转发消息到Kafka
                    let payload = serde_json::to_string(&forward_msg).unwrap();
                    let key = conversation_id_of(&forward_msg);
                    let record: FutureRecord<'_, str, String> = FutureRecord::to(&self.topic)
                        .payload(&payload)
                        .key(&key);

                    match self
                        .kafka
                        .send(record, std::time::Duration::from_secs(10))
                        .await
                    {
                        Ok(_) => {
                            forwarded_message_ids.push(forward_msg.server_id.clone());
                            success_count += 1;
                            debug!(
                                "转发群组消息成功: {} -> {}",
                                original_msg.server_id, forward_msg.server_id
                            );
                        }
                        Err((kafka_error, _)) => {
                            error!("转发群组消息到Kafka失败: {}", kafka_error);
                            error_messages.push(format!("转发群组消息到Kafka失败: {}", kafka_error));
                        }
                    }
                }
            }
        }

        let error_message = if error_messages.is_empty() {
            if success_count == 0 {
                "所有转发都失败了".to_string()
            } else {
                String::new()
            }
        } else {
            error_messages.join("; ")
        };

        Ok(tonic::Response::new(ForwardMessageResponse {
            success: success_count > 0,
            forwarded_message_ids,
            forward_count: success_count,
            error: error_message,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contract::proto::message::{Msg, MsgType};

    #[test]
    fn fill_single_receiver_resolves_peer() {
        // 单聊：会话 ID 反推对端，补齐 receiver_id
        let mut messages = vec![Msg {
            send_id: "u1".into(),
            receiver_id: String::new(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        }];
        fill_single_receiver(&mut messages, "d:u1:u2", "u1");
        assert_eq!(messages[0].receiver_id, "u2");

        // 视角反转：对端仍是 u1
        let mut messages = vec![Msg {
            send_id: "u2".into(),
            receiver_id: String::new(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        }];
        fill_single_receiver(&mut messages, "d:u1:u2", "u2");
        assert_eq!(messages[0].receiver_id, "u1");
    }

    #[test]
    fn fill_single_receiver_skips_group_and_explicit_receiver() {
        // 群聊会话：不反推 receiver_id（群消息由 group_id 定位会话）
        let mut messages = vec![Msg {
            send_id: "u1".into(),
            receiver_id: String::new(),
            msg_type: MsgType::GroupMsg as i32,
            ..Default::default()
        }];
        fill_single_receiver(&mut messages, "g:g1", "u1");
        assert_eq!(messages[0].receiver_id, "");

        // 已有 receiver_id 的消息不被覆盖
        let mut messages = vec![Msg {
            send_id: "u1".into(),
            receiver_id: "keep".into(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        }];
        fill_single_receiver(&mut messages, "d:u1:u2", "u1");
        assert_eq!(messages[0].receiver_id, "keep");
    }

    #[test]
    fn fill_single_receiver_ignores_invalid_conversation() {
        // 非法会话 ID（无对端可解析）：保持原样，不 panic
        let mut messages = vec![Msg {
            send_id: "u1".into(),
            receiver_id: String::new(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        }];
        fill_single_receiver(&mut messages, "g:g1", "u1");
        assert_eq!(messages[0].receiver_id, "");
    }
}
