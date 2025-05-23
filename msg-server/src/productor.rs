use std::time::Duration;

use async_trait::async_trait;
use nanoid::nanoid;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use tonic::transport::Server;
use tracing::{error, info};

use common::config::{AppConfig, Component};
use common::grpc::LoggingInterceptor;
use common::message::chat_service_server::{ChatService, ChatServiceServer};
use common::message::{MsgResponse, MsgType, SendMsgRequest};

/// 消息RPC服务实现
/// 负责接收客户端消息并发送到Kafka消息队列
pub struct ChatRpcService {
    // Kafka生产者实例，用于发送消息到Kafka
    kafka: FutureProducer,
    // Kafka主题名称，消息将被发送到此主题
    topic: String,
}

impl ChatRpcService {
    /// 创建一个新的ChatRpcService实例
    pub fn new(kafka: FutureProducer, topic: String) -> Self {
        Self { kafka, topic }
    }
    
    /// 启动消息服务
    /// 初始化Kafka生产者、确保主题存在、注册服务，并启动RPC服务器
    pub async fn start(config: &AppConfig) {
        // 构建Kafka代理地址字符串
        let broker = config.kafka.hosts.join(",");
        // 配置并创建Kafka生产者
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &broker)
            .set(
                "message.timeout.ms",
                config.kafka.producer.timeout.to_string(),
            )
            .set(
                "socket.timeout.ms",
                config.kafka.connect_timeout.to_string(),
            )
            .set("acks", config.kafka.producer.acks.clone())
            // 确保消息精确发送一次
            .set("enable.idempotence", "true")
            .set("retries", config.kafka.producer.max_retry.to_string())
            .set(
                "retry.backoff.ms",
                config.kafka.producer.retry_interval.to_string(),
            )
            .create()
            .expect("生产者创建失败");

        // 确保Kafka主题存在，如不存在则创建
        Self::ensure_topic_exists(&config.kafka.topic, &broker, config.kafka.connect_timeout as u16)
            .await
            .expect("主题创建失败");

        // 向服务注册中心注册消息服务
        common::grpc_client::base::register_service(config, Component::MessageServer)
            .await
            .expect("服务注册失败");
        info!("<chat> RPC服务已注册到服务注册中心");

        // TODO 创建tonic健康检查服务

        // 创建日志拦截器
        // 用于记录和跟踪所有RPC请求
        let logging_interceptor = LoggingInterceptor::new();

        // 创建聊天RPC服务实例
        let chat_rpc = Self::new(producer, config.kafka.topic.clone());
        // 包装服务并添加日志拦截器
        let service = ChatServiceServer::with_interceptor(chat_rpc, logging_interceptor);
        info!(
            "<chat> RPC服务已启动，监听地址: {}",
            config.rpc.chat.rpc_server_url()
        );

        // 启动RPC服务器，添加健康检查和聊天服务
        Server::builder()
            .add_service(service)
            .serve(config.rpc.chat.rpc_server_url().parse().unwrap())
            .await
            .unwrap();
    }

    /// 确保Kafka主题存在
    /// 如果主题不存在，则创建该主题
    async fn ensure_topic_exists(
        topic_name: &str,
        brokers: &str,
        timeout: u16,
    ) -> Result<(), KafkaError> {
        // 创建Kafka管理客户端
        let admin_client: AdminClient<DefaultClientContext> = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("socket.timeout.ms", timeout.to_string())
            .create()?;

        // 创建新主题的配置
        let new_topics = [NewTopic {
            name: topic_name,
            num_partitions: 1,
            replication: TopicReplication::Fixed(1),
            config: vec![],
        }];

        // 注意：暂时没有找到检查主题是否存在的方法
        // 因此我们尝试创建主题，并通过错误判断主题是否已存在
        // 这种方法虽然不是最优的，但目前可以正常工作
        let options = AdminOptions::new();
        admin_client.create_topics(&new_topics, &options).await?;
        match admin_client.create_topics(&new_topics, &options).await {
            Ok(_) => {
                info!("主题不存在；已创建主题 '{}' ", topic_name);
                Ok(())
            }
            Err(KafkaError::AdminOpCreation(_)) => {
                println!("主题 '{}' 已存在。", topic_name);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }
}

#[async_trait]
impl ChatService for ChatRpcService {
    /// 发送消息到消息队列
    /// 生成消息ID和发送时间，并将消息发送到Kafka
    async fn send_msg(
        &self,
        request: tonic::Request<SendMsgRequest>,
    ) -> Result<tonic::Response<MsgResponse>, tonic::Status> {
        // 从请求中提取消息
        let mut msg = request
            .into_inner()
            .message
            .ok_or(tonic::Status::invalid_argument("消息为空"))?;

        // 为特定类型的消息生成服务器ID
        // 某些系统消息不需要生成新的服务器ID
        if !(msg.msg_type == MsgType::GroupDismissOrExitReceived as i32
            || msg.msg_type == MsgType::GroupInvitationReceived as i32
            || msg.msg_type == MsgType::FriendshipReceived as i32)
        {
            // 使用nanoid生成唯一的消息ID
            msg.server_id = nanoid!();
        }
        // 设置消息发送时间为当前时间戳
        msg.send_time = chrono::Utc::now().timestamp_millis();

        // 将消息序列化为JSON并发送到Kafka
        let payload = serde_json::to_string(&msg).unwrap();
        // 让Kafka自动生成消息键
        let record: FutureRecord<String, String> = FutureRecord::to(&self.topic).payload(&payload);

        info!("将消息发送到Kafka: {:?}", record);
        // 发送消息到Kafka并处理结果
        let err = match self.kafka.send(record, Duration::from_secs(0)).await {
            Ok(_) => String::new(),
            Err((err, msg)) => {
                error!(
                    "发送消息到Kafka失败: {:?}; 原始消息: {:?}",
                    err, msg
                );
                err.to_string()
            }
        };

        // 返回消息响应，包含本地ID、服务器ID、发送时间和错误信息
        Ok(tonic::Response::new(MsgResponse {
            local_id: msg.local_id,
            server_id: msg.server_id,
            send_time: msg.send_time,
            err,
        }))
    }
}
