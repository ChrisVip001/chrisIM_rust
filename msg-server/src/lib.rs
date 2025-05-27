/// msg-server 消息服务器库
/// 
/// 这是即时通讯系统的核心消息处理服务，包含以下主要模块：
/// 
/// ## 核心模块
/// 
/// ### `productor` 模块 - 消息生产者服务
/// - 提供gRPC接口接收客户端消息发送请求
/// - 生成消息唯一ID和时间戳
/// - 将消息发送到Kafka消息队列
/// - 向客户端返回发送结果
/// 
/// ### `consumer` 模块 - 消息消费者服务  
/// - 从Kafka消息队列消费消息
/// - 将消息存储到数据库（PostgreSQL + MongoDB）
/// - 推送消息到在线用户
/// - 处理离线消息存储
/// 
/// ### `storage_service` 模块 - 存储服务接口
/// - 提供消息存储的gRPC接口
/// - 支持消息的保存、查询、删除等操作
/// - 管理消息序列号
/// - 提供健康检查接口
/// 
/// ### `pusher` 模块 - 消息推送服务
/// - 负责将消息推送到在线用户
/// - 通过WebSocket连接发送实时消息
/// - 处理群聊消息的批量推送
/// - 管理用户连接状态
/// 
/// ## 工作流程
/// 
/// ```text
/// 客户端消息 -> ChatRpcService(生产者) -> Kafka队列 -> ConsumerService(消费者) 
///                                                          ↓
/// WebSocket推送 <- PushService <- 存储服务 <- 消息持久化(PostgreSQL + MongoDB)
/// ```
/// 
/// ## 服务特性
/// 
/// - **高可靠性**: 使用Kafka确保消息不丢失
/// - **高性能**: 异步处理，支持大量并发
/// - **可扩展**: 微服务架构，易于水平扩展  
/// - **持久化**: 双存储架构，历史消息和离线消息分离
/// - **实时性**: WebSocket推送，毫秒级消息投递
/// - **监控**: 完整的日志记录和链路追踪
/// 
/// ## 使用示例
/// 
/// ```rust
/// use msg_server::productor::ChatRpcService;
/// use msg_server::consumer::ConsumerService;
/// 
/// // 启动生产者服务
/// ChatRpcService::start(&config).await;
/// 
/// // 启动消费者服务
/// let mut consumer = ConsumerService::new(&config).await?;
/// consumer.consume().await?;
/// ```

pub mod productor;
pub mod consumer;
pub mod storage_service;

/// 推送服务模块
/// 负责将消息推送到在线用户
pub mod pusher {
    pub mod service;
}
