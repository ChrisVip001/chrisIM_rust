use anyhow::Result;
use std::env;
use runtime::config::{AppConfig, Component, ConfigLoader};
use runtime::grpc::LoggingInterceptor;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tonic::transport::Server;
use tracing::info;



#[tokio::main]
async fn main() -> Result<()> {
    // 初始化rustls加密提供程序
    runtime::service::init_rustls();

    // 从环境变量获取配置文件路径
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config/config.yaml".to_string());
    info!("使用配置文件: {}", config_path);
    
    // 使用指定的配置文件路径初始化全局配置
    let app_config = AppConfig::from_file(Some(&config_path))
        .expect(&format!("无法从路径加载配置: {}", config_path));
    // 先用同一份配置构造局部 Arc，main 里不再回读全局单例
    let config = std::sync::Arc::new(app_config.clone());
    ConfigLoader::set_global(app_config);

    // start cleaner
    let _ = msg_storage::clean_receive_box(&config).await;

    // start rpc service
    // DbRpcService::start(&config).await;

    Ok(())
}
