use tracing::{info, Level};

use common::config::{AppConfig, ConfigLoader};
use msg_gateway::ws_server::WsServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载配置文件
    // 初始化全局配置
    ConfigLoader::init_global().expect("初始化全局配置失败");

    // 确保全局配置可以正常访问
    let config = ConfigLoader::get_global().expect("获取全局配置失败");

    // 初始化日志和链路追踪系统
    // 根据配置判断是否启用分布式链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "msg-gateway")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化基本日志系统，不包含链路追踪功能
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
        // 测试消息序列化功能
        let msg = Msg::default();
        println!("{}", serde_json::to_string(&msg).unwrap());
        // 打印RPC服务名称，用于服务注册和发现
        println!(
            "{:?}",
            <MsgServiceServer<rpc::MsgRpcService> as NamedService>::NAME
        );
    }
}
