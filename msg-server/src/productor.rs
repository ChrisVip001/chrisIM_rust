use std::time::Duration;

use async_trait::async_trait;
use nanoid::nanoid;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use tonic::transport::Server;
use tracing::{error, info, warn};
// 添加gRPC健康检查相关导入
use tonic_health::server::HealthReporter;

use common::config::{AppConfig, Component};
use common::grpc::LoggingInterceptor;
use common::proto::message::chat_service_server::{ChatService, ChatServiceServer};
use common::proto::message::{MsgResponse, MsgType, SendMsgRequest};

/// 聊天消息RPC服务实现
/// 
/// 这是msg-server的核心服务之一，负责：
/// 1. 通过gRPC接收来自客户端的消息发送请求
/// 2. 为消息生成唯一的服务器ID和时间戳
/// 3. 将消息序列化后发送到Kafka消息队列
/// 4. 向客户端返回发送结果
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
}

impl ChatRpcService {
    /// 创建一个新的聊天RPC服务实例
    /// 
    /// # 参数
    /// * `kafka` - 已配置的Kafka生产者实例
    /// * `topic` - 消息要发送到的Kafka主题名称
    /// 
    /// # 返回值
    /// 返回ChatRpcService实例
    pub fn new(kafka: FutureProducer, topic: String) -> Self {
        Self { kafka, topic }
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
    pub async fn start(config: &AppConfig) {
        // 构建Kafka代理服务器地址列表
        // 支持多个Kafka节点，用逗号分隔
        let broker = config.kafka.hosts.join(",");
        
        // 配置并创建Kafka生产者
        // 这些配置确保消息的可靠性和性能
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &broker)                    // Kafka集群地址
            .set(
                "message.timeout.ms",
                config.kafka.producer.timeout.to_string(),        // 消息发送超时时间
            )
            .set(
                "socket.timeout.ms",
                config.kafka.connect_timeout.to_string(),         // 连接超时时间
            )
            .set("acks", config.kafka.producer.acks.clone())      // 确认模式(all表示所有副本确认)
            .set("enable.idempotence", "true")                    // 启用幂等性，防止重复消息
            .set("retries", config.kafka.producer.max_retry.to_string())  // 最大重试次数
            .set(
                "retry.backoff.ms",
                config.kafka.producer.retry_interval.to_string(), // 重试间隔时间
            )
            .create()
            .expect("Kafka生产者创建失败");

        // 确保Kafka主题存在，如果不存在则自动创建
        // 这样避免了消息发送到不存在的主题而失败
        if let Err(e) = Self::ensure_topic_exists(&config.kafka.topic, &broker, config.kafka.connect_timeout as u16).await {
            error!("Kafka主题创建失败: {}，但服务将继续运行", e);
            warn!("Kafka服务可能未启动，请检查Kafka服务状态");
        }

        // 向服务注册中心（Consul）注册当前服务
        // 这样其他服务就可以通过服务发现找到这个消息服务
        common::grpc_client::base::register_service(config, Component::MessageServer)
            .await
            .expect("服务注册到Consul失败");
        info!("聊天RPC服务已注册到服务注册中心");

        // 创建gRPC健康检查服务
        // 用于监控服务健康状态，支持Kubernetes等容器编排工具的健康检查
        let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
        
        // 设置服务为健康状态
        // 当服务能正常启动时就认为是健康的
        health_reporter
            .set_serving::<ChatServiceServer<ChatRpcService>>()
            .await;

        // 创建gRPC日志拦截器
        // 用于记录所有RPC请求和响应，便于调试和监控
        let logging_interceptor = LoggingInterceptor::new();

        // 创建聊天RPC服务实例并包装为gRPC服务
        let chat_rpc = Self::new(producer, config.kafka.topic.clone());
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
    ) -> Result<tonic::Response<MsgResponse>, tonic::Status> {
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
        
        // 设置消息的服务器发送时间戳（毫秒级）
        // 这个时间戳是服务器接收到消息的准确时间
        msg.send_time = chrono::Utc::now().timestamp_millis();

        // 将消息对象序列化为JSON字符串
        // JSON格式便于跨语言处理和调试
        let payload = serde_json::to_string(&msg).unwrap();
        
        // 创建Kafka消息记录
        // 不指定分区键，让Kafka自动选择分区
        let record: FutureRecord<String, String> = FutureRecord::to(&self.topic).payload(&payload);

        info!("正在将消息发送到Kafka主题 '{}': 消息ID={}", self.topic, msg.server_id);
        
        // 异步发送消息到Kafka并处理结果
        let err = match self.kafka.send(record, Duration::from_secs(0)).await {
            Ok((partition, offset)) => {
                info!("消息发送成功: 分区={}, 偏移量={}", partition, offset);
                String::new()  // 无错误
            }
            Err((err, _original_msg)) => {
                error!("消息发送到Kafka失败: {:?}", err);
                err.to_string()
            }
        };

        // 构造并返回消息响应
        // 包含本地ID（客户端生成）、服务器ID、发送时间和错误信息
        Ok(tonic::Response::new(MsgResponse {
            local_id: msg.local_id,    // 客户端生成的本地消息ID
            server_id: msg.server_id,  // 服务器生成的全局唯一ID
            send_time: msg.send_time,  // 服务器处理时间戳
            err,                       // 错误信息（空字符串表示成功）
        }))
    }
}
