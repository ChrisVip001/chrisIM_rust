use tracing::info;

use common::config::AppConfig;

use msg_server::productor::ChatRpcService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载配置文件
    // 从指定路径读取系统配置，如果失败则panic
    let config = AppConfig::from_file(Some("./config/config.yaml")).unwrap();
    
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
    
    // 启动消息RPC服务
    // 这是消息服务的核心组件，负责接收客户端消息并处理
    // 包括消息生产者功能、消息存储和转发等
    ChatRpcService::start(&config).await;
    
    // 在程序结束前关闭链路追踪，确保所有数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪...");
        common::logging::shutdown_telemetry();
    }
    
    Ok(())
}
