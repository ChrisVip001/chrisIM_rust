use tracing::info;
use std::env;
use std::sync::Arc;
use runtime::config::AppConfig;

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
/// - 存储服务: 将消息持久化到PostgreSQL（读扩散模型）
/// - 推送服务: 将消息推送到在线用户的WebSocket连接
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化rustls加密提供程序
    // 这是为了确保TLS连接的安全性，用于gRPC和其他加密通信
    runtime::service::init_rustls();
    
    // 从环境变量获取配置文件路径，如果没有设置则使用默认路径
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config/config.yaml".to_string());
    info!("使用配置文件: {}", config_path);
    
    // 加载应用配置文件，该配置包含数据库、消息队列、服务注册等所有配置
    let app_config = AppConfig::from_file(Some(&config_path))
        .unwrap_or_else(|_| panic!("无法从路径加载配置: {}", config_path));
    
    // 设置全局配置，使得整个应用可以访问配置信息
    // 先用同一份配置构造局部 Arc，main 里不再回读全局单例
    let config = std::sync::Arc::new(app_config.clone());

    // 初始化日志和分布式链路追踪系统
    // 根据配置判断是否启用分布式链路追踪功能
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        // 这将启用OpenTelemetry，用于跨服务的链路追踪
        runtime::logging::init_telemetry(&config, "msg-server")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化基本日志系统，不包含链路追踪功能
        runtime::logging::init_from_config(&config, "msg-server")?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }
    
    info!("正在启动消息服务器...");


    
    // 创建并初始化消费者服务实例
    // 消费者服务负责从Kafka消费消息，并进行后续处理
    let consumer_service = ConsumerService::new(&config).await?;
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
    // 同一消费组内运行 N 个消费者，Kafka 自动在它们之间均衡分配分区；
    // 分区数是并行度上限，实际消费者数取配置值与分区数的较小者
    let consumer_service = Arc::new(consumer_service);
    let parallelism = config
        .kafka
        .consumer
        .effective_parallelism()
        .min(config.kafka.effective_partitions() as usize);
    let mut consumer_tasks = Vec::with_capacity(parallelism);
    for _ in 1..parallelism {
        let sibling = Arc::new(consumer_service.fork(&config).await?);
        consumer_tasks.push(tokio::spawn(async move {
            if let Err(e) = sibling.consume().await {
                tracing::error!("消费者服务运行失败: {:?}", e);
            }
        }));
    }
    let first = consumer_service.clone();
    consumer_tasks.push(tokio::spawn(async move {
        if let Err(e) = first.consume().await {
            tracing::error!("消费者服务运行失败: {:?}", e);
        }
    }));

    info!(
        "消息生产者和消费者服务已启动（消费者并行度={}）",
        parallelism
    );

    // 优雅停机：监听 SIGTERM/SIGINT。此前进程完全没有信号处理，
    // 容器滚动发布会硬断 gRPC 服务与 Kafka 消费循环。
    // 消费位点为手动提交，信号到达时未处理完的批次在重启后重新消费
    // （at-least-once + 幂等落库，不丢不重）。
    let shutdown_task = tokio::spawn(runtime::shutdown::wait_for_shutdown_signal());

    // 等待任一服务结束（正常情况下服务会一直运行，除非出现错误）
    // 使用tokio::select!来并发等待多个异步任务
    tokio::select! {
        _ = producer_task => {
            info!("生产者服务已结束");
        }
        _ = &mut consumer_tasks[0] => {
            info!("消费者服务已结束");
        }
        _ = shutdown_task => {
            info!("收到停机信号，msg-server 开始退出（服务实例的健康检查失效后由注册中心自动摘除）");
        }
    }
    
    // 在程序结束前优雅关闭链路追踪，确保所有追踪数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪系统...");
        runtime::logging::shutdown_telemetry();
    }
    
    info!("消息服务器已停止");
    Ok(())
}
