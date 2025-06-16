use std::time::Duration;
use std::sync::Arc;

use async_trait::async_trait;
use nanoid::nanoid;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use tonic::transport::Server;
use tracing::{error, info, debug};

use common::config::{AppConfig, Component};
use common::grpc::LoggingInterceptor;
use common::proto::message::chat_service_server::{ChatService, ChatServiceServer};
use common::proto::message::{MsgType, SendMsgRequest, Msg, MarkMessagesAsReadRequest, MarkMessagesAsReadResponse, MarkConversationAsReadRequest, GetMessageHistoryResponse, GetConversationsRequest, GetConversationsResponse, RevokeMessageRequest, RevokeMessageResponse, DeleteMessagesRequest, DeleteMessagesResponse, ForwardMessageRequest, ForwardMessageResponse, ReplyMessageRequest, ReplyMessageResponse, Conversation, GetDbMessagesRequest};
use msg_storage::{msg_rec_box_repo, message::MsgRecBoxRepo};
use cache::Cache;
use common::Error;

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
    
    /// 消息存储仓库，用于访问MongoDB中的消息数据
    msg_storage: Arc<dyn MsgRecBoxRepo>,
    
    /// 缓存接口，用于获取用户序列号等信息
    cache: Arc<dyn Cache>,
}

impl ChatRpcService {
    /// 创建一个新的聊天RPC服务实例
    /// 
    /// # 参数
    /// * `kafka` - 已配置的Kafka生产者实例
    /// * `topic` - 消息要发送到的Kafka主题名称
    /// * `msg_storage` - 消息存储仓库
    /// * `cache` - 缓存接口
    /// 
    /// # 返回值
    /// 返回ChatRpcService实例
    pub fn new(
        kafka: FutureProducer, 
        topic: String,
        msg_storage: Arc<dyn MsgRecBoxRepo>,
        cache: Arc<dyn Cache>,
    ) -> Self {
        Self { 
            kafka, 
            topic,
            msg_storage,
            cache,
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
    pub async fn start(config: &AppConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 配置Kafka生产者
        let mut kafka_config = ClientConfig::new();
        kafka_config
            .set("bootstrap.servers", &config.kafka.hosts.join(","))
            .set("message.timeout.ms", "30000")
            .set("queue.buffering.max.messages", "100000")
            .set("queue.buffering.max.kbytes", "1048576")
            .set("batch.num.messages", "1000")
            .set("enable.idempotence", "true")
            .set("retries", "2147483647")
            .set("max.in.flight.requests.per.connection", "5")
            .set("acks", "all")
            .set("compression.type", "snappy");

        // 创建Kafka生产者
        let producer: FutureProducer = kafka_config.create().unwrap();

        // 确保Kafka主题存在
        Self::ensure_topic_exists(
            &config.kafka.topic,
            &config.kafka.hosts.join(","),
            config.kafka.connect_timeout as u16,
        )
        .await?;

        info!("Kafka主题 '{}' 已确保存在", config.kafka.topic);

        // 向服务注册中心注册当前服务
        common::grpc_client::base::register_service(config, Component::MessageServer)
            .await
            .unwrap();

        info!("聊天RPC服务已注册到服务注册中心");

        // 初始化消息存储和缓存
        let msg_storage = msg_rec_box_repo(config).await
            .map_err(|e| format!("初始化消息存储失败: {}", e))?;
        let cache = cache::cache(config).await;

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
        let chat_rpc = Self::new(producer, config.kafka.topic.clone(), msg_storage, cache);
        let service = ChatServiceServer::with_interceptor(chat_rpc, logging_interceptor);
        
        info!(
            "聊天RPC服务已启动，监听地址: {}",
            config.rpc.chat.rpc_server_url()
        );

        // 启动gRPC服务器
        // 同时提供健康检查服务和聊天消息服务
        Server::builder()
            .add_service(health_service)     // 健康检查服务
            .add_service(service)            // 聊天消息服务
            .serve(config.rpc.chat.rpc_server_url().parse().unwrap())
            .await
            .unwrap();
        
        Ok(())
    }

    /// 确保Kafka主题存在
    /// 
    /// 检查指定的Kafka主题是否存在，如果不存在则创建该主题。
    /// 这是一个重要的初始化步骤，确保消息有地方可以发送。
    /// 
    /// # 参数
    /// * `topic_name` - 要检查/创建的主题名称
    /// * `brokers` - Kafka代理服务器地址
    /// * `timeout` - 连接超时时间（毫秒）
    /// 
    /// # 返回值
    /// * `Ok(())` - 主题存在或创建成功
    /// * `Err(KafkaError)` - 创建主题失败
    async fn ensure_topic_exists(
        topic_name: &str,
        brokers: &str,
        timeout: u16,
    ) -> Result<(), KafkaError> {
        // 创建Kafka管理客户端用于管理主题
        let admin_client: AdminClient<DefaultClientContext> = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("socket.timeout.ms", timeout.to_string())
            .create()?;

        // 定义新主题的配置
        // 设置1个分区和1个副本（适合开发环境，生产环境建议更多）
        let new_topics = [NewTopic {
            name: topic_name,                               // 主题名称
            num_partitions: 1,                              // 分区数量
            replication: TopicReplication::Fixed(1),       // 副本数量
            config: vec![],                                 // 额外配置（使用默认）
        }];

        // 尝试创建主题
        // 注意：这里采用"先创建后判断"的策略，因为检查主题存在性的API较复杂
        let options = AdminOptions::new();
        match admin_client.create_topics(&new_topics, &options).await {
            Ok(_) => {
                info!("Kafka主题 '{}' 不存在，已成功创建", topic_name);
                Ok(())
            }
            Err(KafkaError::AdminOpCreation(_)) => {
                // 这种错误通常表示主题已存在
                info!("Kafka主题 '{}' 已存在，无需创建", topic_name);
                Ok(())
            }
            Err(err) => {
                error!("创建Kafka主题失败: {:?}", err);
                Err(err)
            }
        }
    }
}

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

        // 为特定类型的消息生成服务器ID
        // 某些系统消息（如群组解散、好友邀请等）可能已经有了服务器ID，无需重新生成
        if !(msg.msg_type == MsgType::GroupDismissOrExitReceived as i32
            || msg.msg_type == MsgType::GroupInvitationReceived as i32
            || msg.msg_type == MsgType::FriendshipReceived as i32)
        {
            // 使用nanoid生成URL安全的唯一消息ID
            // nanoid比UUID更短且更安全
            msg.server_id = nanoid!();
        }
        
        // 生成发送序列号
        // 所有用户发送的消息都需要一个唯一的发送序列号用于排序和去重
        if msg.send_seq == 0 {
            match self.cache.incr_send_seq(&msg.send_id).await {
                Ok((seq, _, _)) => {
                    debug!("为用户 {} 生成发送序列号: {}", msg.send_id, seq);
                    msg.send_seq = seq;
                }
                Err(e) => {
                    error!("生成发送序列号失败: {}", e);
                    return Err(tonic::Status::internal(format!("生成发送序列号失败: {}", e)));
                }
            }
        }
        
        // 设置消息的服务器发送时间戳（毫秒级）
        msg.send_time = chrono::Utc::now().timestamp_millis();

        // 将消息对象序列化为JSON字符串
        let payload = serde_json::to_string(&msg).unwrap();
        
        // 创建Kafka消息记录
        let record: FutureRecord<'_, (), String> = FutureRecord::to(&self.topic).payload(&payload);

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
                Err(tonic::Status::internal(format!("消息发送失败: {}", kafka_error)))
            }
        }
    }

    /// 标记消息已读
    async fn mark_messages_as_read(
        &self,
        request: tonic::Request<MarkMessagesAsReadRequest>,
    ) -> Result<tonic::Response<MarkMessagesAsReadResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!("标记消息已读请求: user_id={}, msg_seqs={:?}", req.user_id, req.msg_seqs);

        if req.msg_seqs.is_empty() {
            return Err(tonic::Status::invalid_argument("消息序列号列表不能为空"));
        }

        // 调用消息存储服务标记消息已读
        match self.msg_storage.msg_read(&req.user_id, &req.msg_seqs).await {
            Ok(()) => {
                debug!("成功标记 {} 条消息为已读", req.msg_seqs.len());
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
        debug!("标记会话已读请求: user_id={}, conversation_id={}, up_to_time={:?}", 
               req.user_id, req.conversation_id, req.up_to_time);

        // 验证参数
        if req.user_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID不能为空"));
        }

        if req.conversation_id.is_empty() {
            return Err(tonic::Status::invalid_argument("会话ID不能为空"));
        }

        // 调用消息存储服务标记会话已读
        match self.msg_storage.mark_conversation_read(&req.user_id, &req.conversation_id, req.up_to_time).await {
            Ok(count) => {
                debug!("成功标记会话 {} 中 {} 条消息为已读", req.conversation_id, count);
                Ok(tonic::Response::new(MarkMessagesAsReadResponse {
                    success: true,
                    read_count: count,
                    error: String::new(),
                }))
            }
            Err(e) => {
                error!("标记会话已读失败: {}", e);
                Err(tonic::Status::internal(format!("标记会话已读失败: {}", e)))
            }
        }
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

        // 获取用户当前序列号
        let (current_seq, current_send_seq) = match self.cache.get_cur_seq(&req.user_id).await {
            Ok(seqs) => seqs,
            Err(e) => {
                error!("获取用户序列号失败: {}", e);
                return Err(tonic::Status::internal(format!("获取用户序列号失败: {}", e)));
            }
        };

        // 确定实际的序列号范围
        // 如果传入的结束序列号为0，则使用当前序列号
        let actual_send_start = req.send_seq_start;
        let actual_send_end = if req.send_seq_end == 0 { current_send_seq } else { req.send_seq_end };
        let actual_seq_start = req.seq_start;
        let actual_seq_end = if req.seq_end == 0 { current_seq } else { req.seq_end };

        debug!("实际查询范围: send_seq: {}-{}, seq: {}-{}", 
               actual_send_start, actual_send_end, actual_seq_start, actual_seq_end);

        // 使用新的按会话ID查询方法
        match self.msg_storage.get_conversation_messages_by_seq_range(
            &req.user_id, 
            &req.conversation_id, 
            actual_send_start, 
            actual_send_end, 
            actual_seq_start, 
            actual_seq_end
        ).await {
            Ok(messages) => {
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
        debug!("获取会话列表请求，用户ID: {}, 同步模式: {:?}", req.user_id, req.sync_mode);

        // 获取用户当前序列号
        let (rec_seq, send_seq) = self.cache.get_cur_seq(&req.user_id).await?;

        // 确定查询模式和范围
        let query_range = self.determine_query_range(&req, rec_seq, send_seq);
        debug!("查询模式: {:?}, 范围: {:?}", query_range.mode, query_range);

        // 如果是增量同步且无新消息，直接返回
        if query_range.mode == QueryMode::IncrementalSync && query_range.is_empty() {
            debug!("增量同步：没有新消息");
            return Ok(tonic::Response::new(GetConversationsResponse {
                conversations: Vec::new(),
                total: 0,
                seq_max: rec_seq,
                send_seq_max: send_seq,
            }));
        }

        // 查询消息
        match self.fetch_messages(&req.user_id, &query_range).await {
            Ok(messages) => {
                debug!("获取到 {} 条消息", messages.len());
                let conversations = self.build_conversations(&req, messages);
                debug!("构建了 {} 个会话", conversations.len());
                let total = conversations.len() as i32;

                Ok(tonic::Response::new(GetConversationsResponse {
                    conversations,
                    total,
                    seq_max: rec_seq,
                    send_seq_max: send_seq,
                }))
            }
            Err(e) => {
                error!("获取会话列表失败: {}", e);
                Err(tonic::Status::internal(format!("获取会话列表失败: {}", e)))
            }
        }
    }

    /// 撤回消息
    async fn revoke_message(
        &self,
        request: tonic::Request<RevokeMessageRequest>,
    ) -> Result<tonic::Response<RevokeMessageResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!("撤回消息请求: user_id={}, message_id={}", req.user_id, req.message_id);

        // 验证参数
        if req.user_id.is_empty() || req.message_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID和消息ID不能为空"))
        }

        // 首先检查消息是否存在以及撤回时间限制
        match self.msg_storage.get_message(&req.message_id).await {
            Ok(Some(msg)) => {
                // 验证消息是否属于当前用户
                if msg.send_id != req.user_id {
                    return Err(tonic::Status::internal("只能撤回自己发送的消息"))
                }

                // 验证消息是否已经被撤回
                if msg.is_revoked {
                    return Err(tonic::Status::not_found("消息已经被撤回"))
                }

                // 验证撤回时间限制（2分钟内）
                let now = chrono::Utc::now().timestamp_millis();
                let time_limit = 2 * 60 * 1000; // 2分钟
                if now - msg.send_time > time_limit {
                    return Err(tonic::Status::internal("消息发送超过2分钟，无法撤回"))
                }

                // 执行撤回操作
                match self.msg_storage.revoke_message(&req.user_id, &req.message_id).await {
                    Ok(()) => {
                        debug!("消息撤回成功: {}", req.message_id);
                        
                        // TODO: 发送撤回通知给相关用户
                        // 这里可以发送一条撤回通知消息到Kafka，让其他用户知道消息被撤回了
                        
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
            Ok(None) => {
                Err(tonic::Status::not_found("消息不存在"))
            }
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
        debug!("删除消息请求: user_id={}, message_ids={:?}, message_seqs={:?}", 
               req.user_id, req.message_ids, req.message_seqs);

        // 验证参数
        if req.user_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID不能为空"))
        }

        if req.message_ids.is_empty() && req.message_seqs.is_empty() {
            return Err(tonic::Status::invalid_argument("必须提供消息ID或消息序列号"))
        }

        let mut total_deleted = 0i32;

        // 按消息ID删除
        if !req.message_ids.is_empty() {
            match self.msg_storage.delete_messages_by_ids(&req.user_id, &req.message_ids).await {
                Ok(count) => {
                    total_deleted += count;
                    debug!("按消息ID删除了 {} 条消息", count);
                }
                Err(e) => {
                    error!("按消息ID删除消息失败: {}", e);
                    return Err(tonic::Status::internal(format!("删除消息失败: {}", e)))
                }
            }
        }

        // 按消息序列号删除
        if !req.message_seqs.is_empty() {
            let seqs_len = req.message_seqs.len();
            match self.msg_storage.delete_messages(&req.user_id, req.message_seqs).await {
                Ok(()) => {
                    // delete_messages方法不返回删除数量，我们假设全部删除成功
                    total_deleted += seqs_len as i32;
                    debug!("按序列号删除了 {} 条消息", seqs_len);
                }
                Err(e) => {
                    error!("按序列号删除消息失败: {}", e);
                    return Err(tonic::Status::internal(format!("删除消息失败: {}", e)))
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
        debug!("转发消息请求: user_id={}, original_message_id={}, targets={:?}/{:?}", 
               req.user_id, req.original_message_id, req.target_user_ids, req.target_group_ids);

        // 验证参数
        if req.user_id.is_empty() || req.original_message_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID和原始消息ID不能为空"))
        }

        if req.target_user_ids.is_empty() && req.target_group_ids.is_empty() {
            return Err(tonic::Status::invalid_argument("必须指定转发目标"))
        }

        // 获取原始消息
        let original_msg = match self.msg_storage.get_message(&req.original_message_id).await {
            Ok(Some(msg)) => msg,
            Ok(None) => {
                return Err(tonic::Status::not_found("原始消息不存在"))
            }
            Err(e) => {
                error!("获取原始消息失败: {}", e);
                return Err(tonic::Status::internal("获取原始消息失败"))
            }
        };

        let mut forwarded_message_ids = Vec::new();
        let mut success_count = 0;

        // 转发给单聊用户
        for target_user_id in &req.target_user_ids {
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
                forward_comment: req.forward_comment.clone(),
                is_forwarded: true,
                is_reply: false,
            };

            // 发送转发消息到Kafka
            let payload = serde_json::to_string(&forward_msg).unwrap();
            let record: FutureRecord<'_, (), String> = FutureRecord::to(&self.topic).payload(&payload);

            match self.kafka.send(record, Duration::from_secs(10)).await {
                Ok(_) => {
                    forwarded_message_ids.push(forward_msg.server_id.clone());
                    success_count += 1;
                    debug!("转发消息成功: {} -> {}", req.original_message_id, forward_msg.server_id);
                }
                Err((kafka_error, _)) => {
                    error!("转发消息到Kafka失败: {}", kafka_error);
                }
            }
        }

        // 转发给群组
        for target_group_id in &req.target_group_ids {
            let forward_msg = Msg {
                send_id: req.user_id.clone(),
                receiver_id: target_group_id.clone(),
                local_id: format!("forward_{}", nanoid::nanoid!()),
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
                forward_comment: req.forward_comment.clone(),
                is_forwarded: true,
                is_reply: false,
            };

            // 发送转发消息到Kafka
            let payload = serde_json::to_string(&forward_msg).unwrap();
            let record: FutureRecord<'_, (), String> = FutureRecord::to(&self.topic).payload(&payload);

            match self.kafka.send(record, std::time::Duration::from_secs(10)).await {
                Ok(_) => {
                    forwarded_message_ids.push(forward_msg.server_id.clone());
                    success_count += 1;
                    debug!("转发群组消息成功: {} -> {}", req.original_message_id, forward_msg.server_id);
                }
                Err((kafka_error, _)) => {
                    error!("转发群组消息到Kafka失败: {}", kafka_error);
                }
            }
        }

        Ok(tonic::Response::new(ForwardMessageResponse {
            success: success_count > 0,
            forwarded_message_ids,
            forward_count: success_count,
            error: if success_count == 0 { "所有转发都失败了".to_string() } else { String::new() },
        }))
    }

    /// 回复消息
    async fn reply_message(
        &self,
        request: tonic::Request<ReplyMessageRequest>,
    ) -> Result<tonic::Response<ReplyMessageResponse>, tonic::Status> {
        let req = request.into_inner();
        debug!("回复消息请求: user_id={}, original_message_id={}, reply_content={}", 
               req.user_id, req.original_message_id, req.reply_content);

        // 验证参数
        if req.user_id.is_empty() || req.original_message_id.is_empty() {
            return Err(tonic::Status::invalid_argument("用户ID和原始消息ID不能为空"));
        }

        if req.reply_content.trim().is_empty() {
            return Err(tonic::Status::invalid_argument("回复内容不能为空"));
        }

        if req.reply_content.len() > 2048 {
            return Err(tonic::Status::invalid_argument("回复内容不能超过2048字符"));
        }

        // 获取原始消息以确定回复目标
        let original_msg = match self.msg_storage.get_message(&req.original_message_id).await {
            Ok(Some(msg)) => msg,
            Ok(None) => {
                return Err(tonic::Status::not_found("原始消息不存在"));
            }
            Err(e) => {
                error!("获取原始消息失败: {}", e);
                return Err(tonic::Status::internal(format!("获取原始消息失败: {}", e)));
            }
        };

        // 确定回复目标和消息类型
        let (receiver_id, msg_type, group_id) = if !original_msg.group_id.is_empty() {
            // 群聊回复
            (original_msg.group_id.clone(), MsgType::GroupMsg as i32, original_msg.group_id.clone())
        } else {
            // 单聊回复：如果原消息是别人发给我的，回复给发送者；如果是我发给别人的，回复给接收者
            let target_id = if original_msg.send_id == req.user_id {
                original_msg.receiver_id.clone()
            } else {
                original_msg.send_id.clone()
            };
            (target_id, MsgType::SingleMsg as i32, String::new())
        };

        // 如果提供了conversation_id，使用它来覆盖自动推断的接收者
        let final_receiver_id = req.conversation_id.unwrap_or(receiver_id);

        // 构建回复消息
        let reply_msg = Msg {
            send_id: req.user_id.clone(),
            receiver_id: final_receiver_id,
            local_id: format!("reply_{}", nanoid::nanoid!()),
            server_id: nanoid::nanoid!(),
            create_time: chrono::Utc::now().timestamp_millis(),
            send_time: chrono::Utc::now().timestamp_millis(),
            seq: 0,
            send_seq: 0,
            msg_type,
            content_type: req.content_type,
            content: req.reply_content.into_bytes(),
            is_read: false,
            group_id,
            platform: 0, // 默认平台
            avatar: String::new(),
            nickname: String::new(),
            related_msg_id: Some(req.original_message_id.clone()),
            is_revoked: false,
            revoke_time: 0,
            revoked_by: String::new(),
            forward_comment: None,
            is_forwarded: false,
            is_reply: true,
        };

        // 发送回复消息到Kafka
        let payload = serde_json::to_string(&reply_msg).unwrap();
        let record: FutureRecord<'_, (), String> = FutureRecord::to(&self.topic).payload(&payload);

        match self.kafka.send(record, std::time::Duration::from_secs(10)).await {
            Ok(_) => {
                debug!("回复消息成功: {} -> {}", req.original_message_id, reply_msg.server_id);
                Ok(tonic::Response::new(ReplyMessageResponse {
                    success: true,
                    reply_message_id: reply_msg.server_id,
                    send_time: reply_msg.send_time,
                    error: String::new(),
                }))
            }
            Err((kafka_error, _)) => {
                error!("回复消息到Kafka失败: {}", kafka_error);
                Err(tonic::Status::internal(format!("发送回复消息失败: {}", kafka_error)))
            }
        }
    }
}

/// 查询模式枚举
#[derive(Debug, PartialEq)]
enum QueryMode {
    /// 增量同步：获取指定序列号之后的新消息
    IncrementalSync,
    /// 全量离线：获取所有离线消息
    FullOffline,
    /// 正常模式：获取最近N条消息
    Normal,
}

/// 查询范围
#[derive(Debug)]
struct QueryRange {
    mode: QueryMode,
    rec_start: i64,
    rec_end: i64,
    send_start: i64,
    send_end: i64,
}

impl QueryRange {
    fn is_empty(&self) -> bool {
        self.rec_start > self.rec_end && self.send_start > self.send_end
    }
}

impl ChatRpcService {
    /// 确定查询模式和范围
    fn determine_query_range(&self, req: &GetConversationsRequest, rec_seq: i64, send_seq: i64) -> QueryRange {
        let sync_mode = req.sync_mode.unwrap_or(false);
        let since_seq = req.since_seq.unwrap_or(0);
        let since_send_seq = req.since_send_seq.unwrap_or(0);

        // 判断模式
        let mode = if sync_mode && (since_seq > 0 || since_send_seq > 0) {
            QueryMode::IncrementalSync
        } else if sync_mode || (since_seq == 0 && since_send_seq == 0) {
            QueryMode::FullOffline
        } else {
            QueryMode::Normal
        };

        // 根据模式计算查询范围
        match mode {
            QueryMode::IncrementalSync => {
                // 增量同步：从客户端最后序列号+1开始到当前序列号
                QueryRange {
                    mode,
                    rec_start: since_seq + 1,
                    rec_end: rec_seq,
                    send_start: since_send_seq + 1,
                    send_end: send_seq,
                }
            }
            QueryMode::FullOffline => {
                // 全量离线：获取最近的消息，限制数量防止内存溢出
                const OFFLINE_RECENT_COUNT: i64 = 5000; // 离线模式最多获取5000条消息
                QueryRange {
                    mode,
                    rec_start: std::cmp::max(1, rec_seq - OFFLINE_RECENT_COUNT),
                    rec_end: rec_seq,
                    send_start: std::cmp::max(1, send_seq - OFFLINE_RECENT_COUNT),
                    send_end: send_seq,
                }
            }
            QueryMode::Normal => {
                // 正常模式：获取最近2000条消息
                const RECENT_COUNT: i64 = 2000;
                QueryRange {
                    mode,
                    rec_start: std::cmp::max(1, rec_seq - RECENT_COUNT),
                    rec_end: rec_seq,
                    send_start: std::cmp::max(1, send_seq - RECENT_COUNT),
                    send_end: send_seq,
                }
            }
        }
    }

    /// 查询消息
    async fn fetch_messages(&self, user_id: &str, range: &QueryRange) -> Result<Vec<Msg>, Error> {
        self.msg_storage
            .get_msgs(user_id, range.send_start, range.send_end, range.rec_start, range.rec_end)
            .await
            .map_err(|e| e.into())
    }

    /// 构建会话列表
    fn build_conversations(&self, req: &GetConversationsRequest, messages: Vec<Msg>) -> Vec<Conversation> {
        if messages.is_empty() {
            return Vec::new();
        }

        // 按会话ID分组消息
        let conversations_map = self.group_messages_by_conversation(&req.user_id, messages);
        
        // 构建会话对象
        let mut conversations: Vec<Conversation> = conversations_map
            .into_iter()
            .filter_map(|(conversation_id, mut msgs)| {
                if msgs.is_empty() {
                    return None;
                }

                // 按发送时间排序（最新的在后）
                msgs.sort_by(|a, b| a.send_time.cmp(&b.send_time));

                Some(self.create_conversation(conversation_id, msgs))
            })
            .collect();

        // 按最后活跃时间排序（最新的在后）
        conversations.sort_by(|a, b| a.last_active_time.cmp(&b.last_active_time));
        conversations
    }

    /// 按会话ID分组消息
    fn group_messages_by_conversation(&self, user_id: &str, messages: Vec<Msg>) -> std::collections::HashMap<String, Vec<Msg>> {
        let mut conversations = std::collections::HashMap::new();

        for msg in messages {
            let conversation_id = self.extract_conversation_id(&msg, user_id);
            if !conversation_id.is_empty() {
                conversations.entry(conversation_id).or_insert_with(Vec::new).push(msg);
            }
        }

        conversations
    }

    /// 提取会话ID
    fn extract_conversation_id(&self, msg: &Msg, user_id: &str) -> String {
        if msg.msg_type == MsgType::GroupMsg as i32 {
            // 群聊：会话ID是群组ID
            msg.group_id.clone()
        } else {
            // 单聊：会话ID是对方的用户ID
            if msg.send_id == user_id {
                msg.receiver_id.clone()
            } else {
                msg.send_id.clone()
            }
        }
    }

    /// 创建会话对象
    fn create_conversation(&self, conversation_id: String, msgs: Vec<Msg>) -> Conversation {
        let conversation_type = if msgs[0].msg_type == MsgType::GroupMsg as i32 {
            "group"
        } else {
            "single"
        }.to_string();

        // 计算未读消息数（只计算接收到的未读消息）
        let unread_count = msgs.iter()
            .filter(|msg| !msg.is_read && msg.receiver_id != msg.send_id)
            .count() as i32;

        let last_active_time = msgs[0].send_time;
        
        Conversation {
            conversation_id,
            conversation_type,
            recent_messages: msgs,
            unread_count,
            last_active_time,
        }
    }
}
