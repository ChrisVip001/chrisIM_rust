use tracing::{info, Level};

use common::config::AppConfig;
use msg_gateway::ws_server::WsServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载配置
    let config = AppConfig::from_file(Some("./config/config.yaml")).unwrap();
    
    // 初始化日志和链路追踪
    // 根据配置判断是否启用链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "msg-gateway")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化日志系统
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }
    
    info!("正在启动WebSocket网关服务...");
    
    // 启动WebSocket服务器
    WsServer::start(config).await;
    
    // 在程序结束前关闭链路追踪，确保所有数据都被发送
    info!("正在关闭链路追踪...");
    common::logging::shutdown_telemetry();
    
    Ok(())
}
#[cfg(test)]
mod tests {
    use common::message::msg_service_server::MsgServiceServer;
    use common::message::Msg;
    use msg_gateway::rpc;
    use tonic::server::NamedService;

    #[test]
    fn test_load() {
        let msg = Msg::default();
        println!("{}", serde_json::to_string(&msg).unwrap());
        println!(
            "{:?}",
            <MsgServiceServer<rpc::MsgRpcService> as NamedService>::NAME
        );
    }
}
