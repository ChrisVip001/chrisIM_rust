use anyhow::Result;
use std::env;
use common::config::{AppConfig, Component, ConfigLoader};
use common::grpc::LoggingInterceptor;
use common::proto::msg_storage::message_storage_server::MessageStorageServer;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info};

mod api;
mod errors;
mod model;
mod repository;
mod service;

use service::MessageStorageServiceImpl;

// 导入消息存储服务proto文件描述符，用于gRPC反射
const FILE_DESCRIPTOR_SET: &[u8] = common::proto::msg_storage::FILE_DESCRIPTOR_SET;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化rustls加密提供程序
    common::service::init_rustls();

    // 从环境变量获取配置文件路径
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config/config.yaml".to_string());
    info!("使用配置文件: {}", config_path);
    
    // 使用指定的配置文件路径初始化全局配置
    let app_config = AppConfig::from_file(Some(&config_path))
        .expect(&format!("无法从路径加载配置: {}", config_path));
    ConfigLoader::set_global(app_config);

    // 确保全局配置可以正常访问
    let config = ConfigLoader::get_global().expect("获取全局配置失败");

    // start cleaner
    msg_storage::clean_receive_box(&config).await;

    // start rpc service
    // DbRpcService::start(&config).await;

    Ok(())
}
