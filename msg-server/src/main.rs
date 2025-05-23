use tracing::info;

use common::config::{ConfigLoader};

use msg_server::productor::ChatRpcService;
use msg_server::consumer::ConsumerService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化rustls加密提供程序
    common::service::init_rustls();
    
    // 初始化全局配置
    ConfigLoader::init_global().expect("初始化全局配置失败");

    // 确保全局配置可以正常访问
    let config = ConfigLoader::get_global().expect("获取全局配置失败");


    // 初始化日志和链路追踪系统
    // 根据配置判断是否启用分布式链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "msg-server")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化基本日志系统，不包含链路追踪功能
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }
    
    info!("正在启动消息服务...");
    
    // 创建消费者服务实例
    let mut consumer_service = ConsumerService::new(&config).await?;
    info!("消费者服务已初始化");
    
    // 克隆配置以便在异步任务中使用
    let config_clone = config.clone();
    
    // 同时启动生产者和消费者服务
    let producer_task = tokio::spawn(async move {
        ChatRpcService::start(&config_clone).await;
    });
    
    let consumer_task = tokio::spawn(async move {
        if let Err(e) = consumer_service.consume().await {
            tracing::error!("消费者服务运行失败: {:?}", e);
        }
    });
    
    info!("生产者和消费者服务已启动");
    
    // 等待任一服务结束（通常不会结束，除非出错）
    tokio::select! {
        _ = producer_task => {
            info!("生产者服务已结束");
        }
        _ = consumer_task => {
            info!("消费者服务已结束");
        }
    }
    
    // 在程序结束前关闭链路追踪，确保所有数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪...");
        common::logging::shutdown_telemetry();
    }
    
    Ok(())
}
