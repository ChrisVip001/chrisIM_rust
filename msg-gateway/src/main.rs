use tracing::info;

use common::config::ConfigLoader;
use msg_gateway::ws_server::WsServer;

/// msg-gateway 主程序入口
/// 
/// 这是即时通讯系统的WebSocket网关服务，负责：
/// 1. 处理客户端WebSocket连接
/// 2. 验证用户身份和权限
/// 3. 管理在线用户连接状态
/// 4. 接收并推送实时消息
/// 5. 与msg-server进行gRPC通信
/// 
/// 服务架构:
/// - WebSocket服务器: 处理客户端连接和消息收发
/// - gRPC服务器: 接收来自msg-server的消息推送请求
/// - 连接管理器: 管理所有客户端连接的生命周期
/// - 服务注册: 向Consul注册服务供其他服务发现
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化rustls加密提供程序
    // 这是为了确保TLS连接的安全性，用于gRPC和其他加密通信
    common::service::init_rustls();

    // 从环境变量获取配置文件路径，如果没有设置则使用默认路径
    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "./config/config.yaml".to_string());
    info!("使用配置文件: {}", config_path);
    
    // 使用指定的配置文件路径初始化全局配置
    // 配置包含WebSocket服务器、gRPC服务器、JWT密钥等所有必要信息
    let app_config = common::config::AppConfig::from_file(Some(&config_path))
        .expect(&format!("无法从路径加载配置: {}", config_path));
    ConfigLoader::set_global(app_config);

    // 确保全局配置可以正常访问
    let config = ConfigLoader::get_global().expect("获取全局配置失败");

    // 初始化日志和链路追踪系统
    // 根据配置判断是否启用分布式链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        // 这将启用OpenTelemetry，用于跨服务的链路追踪
        common::logging::init_telemetry(&config, "msg-gateway")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化基本日志系统，不包含链路追踪功能
        common::logging::init_from_config(&config, "msg-gateway")?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }
    
    info!("正在启动WebSocket网关服务...");
    info!("服务功能说明:");
    info!("  1. WebSocket服务器: 处理客户端连接，支持多平台同时在线");
    info!("  2. gRPC服务器: 接收msg-server的消息推送请求");
    info!("  3. 连接管理: 自动心跳检测、JWT认证、连接状态管理");
    info!("  4. 消息路由: 智能分发单聊和群聊消息");
    
    // 启动WebSocket服务器
    // 这会同时启动WebSocket服务器和gRPC服务器
    WsServer::start(config).await;
    
    // 在程序结束前关闭链路追踪，确保所有数据都被发送
    info!("正在关闭链路追踪...");
    common::logging::shutdown_telemetry();
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use common::proto::message::msg_service_server::MsgServiceServer;
    use common::proto::message::Msg;
    use msg_gateway::rpc;
    use tonic::server::NamedService;

    #[test]
    fn test_load() {
        // 测试消息序列化功能
        // 验证消息对象能够正确序列化为JSON格式
        let msg = Msg::default();
        println!("{}", serde_json::to_string(&msg).unwrap());
        
        // 打印RPC服务名称，用于服务注册和发现
        // 这个名称会在Consul中注册，供其他服务调用
        println!(
            "{:?}",
            <MsgServiceServer<rpc::MsgRpcService> as NamedService>::NAME
        );
    }
}
