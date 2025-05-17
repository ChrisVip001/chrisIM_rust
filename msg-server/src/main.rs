use tracing::info;

use common::config::AppConfig;

use msg_server::productor::ChatRpcService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载配置
    let config = AppConfig::from_file(Some("./config/config.yaml")).unwrap();
    
    // 初始化日志 - 从配置文件初始化
    common::logging::init_from_config(&config)?;
    
    info!("正在启动消息服务...");
    
    ChatRpcService::start(&config).await;
    Ok(())
}
