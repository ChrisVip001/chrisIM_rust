use tracing::info;
use std::env;
use common::config::{ConfigLoader, AppConfig};

use msg_server::productor::ChatRpcService;
use msg_server::consumer::ConsumerService;

/// msg-server 主程序入口
/// 
/// 这是即时通讯系统的消息服务器，负责：
/// 1. 接收来自客户端的消息请求（生产者服务）
/// 2. 消费Kafka消息队列中的消息并处理（消费者服务）
/// 3. 将消息存储到数据库
/// 4. 推送消息到在线用户
/// 
/// 服务架构:
/// - 生产者服务(ChatRpcService): 通过gRPC接收消息，生成消息ID，发送到Kafka
/// - 消费者服务(ConsumerService): 从Kafka消费消息，存储并推送给用户
/// - 存储服务: 将消息持久化到PostgreSQL和MongoDB
/// - 推送服务: 将消息推送到在线用户的WebSocket连接
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化rustls加密提供程序
    // 这是为了确保TLS连接的安全性，用于gRPC和其他加密通信
    common::service::init_rustls();
    
    // 从环境变量获取配置文件路径，如果没有设置则使用默认路径
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config/config.yaml".to_string());
    info!("使用配置文件: {}", config_path);
    
    // 加载应用配置文件，该配置包含数据库、消息队列、服务注册等所有配置
    let app_config = AppConfig::from_file(Some(&config_path))
        .expect(&format!("无法从路径加载配置: {}", config_path));
    
    // 设置全局配置，使得整个应用可以访问配置信息
    ConfigLoader::set_global(app_config);
    
    // 获取全局配置实例，后续所有服务都会使用这个配置
    let config = ConfigLoader::get_global().expect("获取全局配置失败");

    // 初始化日志和分布式链路追踪系统
    // 根据配置判断是否启用分布式链路追踪功能
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        // 这将启用OpenTelemetry，用于跨服务的链路追踪
        common::logging::init_telemetry(&config, "msg-server")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化基本日志系统，不包含链路追踪功能
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }
    
    info!("正在启动消息服务器...");
    
    // 创建并初始化消费者服务实例
    // 消费者服务负责从Kafka消费消息，并进行后续处理
    let mut consumer_service = ConsumerService::new(&config).await?;
    info!("消费者服务已初始化");
    
    // 克隆配置以便在异步任务中使用
    // Rust的所有权系统要求在异步任务中使用配置时需要克隆
    let config_clone = config.clone();
    
    // 启动生产者服务（ChatRpcService）
    // 该服务通过gRPC对外提供消息发送接口
    let producer_task = tokio::spawn(async move {
        if let Err(e) = ChatRpcService::start(&config_clone).await {
            tracing::error!("生产者服务启动失败: {:?}", e);
        }
    });
    
    // 启动消费者服务
    // 该服务从Kafka消费消息，处理消息存储和推送
    let consumer_task = tokio::spawn(async move {
        if let Err(e) = consumer_service.consume().await {
            tracing::error!("消费者服务运行失败: {:?}", e);
        }
    });
    
    info!("消息生产者和消费者服务已启动");
    info!("系统架构说明:");
    info!("  1. 生产者服务: 接收客户端消息请求，生成消息ID，发送到Kafka队列");
    info!("  2. 消费者服务: 从Kafka消费消息，存储到数据库，推送给在线用户");
    info!("  3. 存储层: PostgreSQL存储历史消息，MongoDB存储离线消息盒子");
    info!("  4. 推送层: 通过WebSocket推送消息到在线用户");
    
    // 等待任一服务结束（正常情况下服务会一直运行，除非出现错误）
    // 使用tokio::select!来并发等待多个异步任务
    tokio::select! {
        _ = producer_task => {
            info!("生产者服务已结束");
        }
        _ = consumer_task => {
            info!("消费者服务已结束");
        }
    }
    
    // 在程序结束前优雅关闭链路追踪，确保所有追踪数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪系统...");
        common::logging::shutdown_telemetry();
    }
    
    info!("消息服务器已停止");
    Ok(())
}
