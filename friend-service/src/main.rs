use anyhow::Result;
use common::config::{AppConfig, Component, ConfigLoader};
use common::grpc::LoggingInterceptor;
use common::grpc_client::base::register_service;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info};

mod model;
mod repository;
mod service;

use common::proto::friend::friend_service_server::FriendServiceServer;
use service::friend_service::FriendServiceImpl;
// 导入好友服务proto文件描述符，用于gRPC反射
const FILE_DESCRIPTOR_SET: &[u8] = common::proto::friend::FILE_DESCRIPTOR_SET;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化rustls加密提供程序
    common::service::init_rustls();

    // 加载配置
    info!("初始化全局配置单例");
    ConfigLoader::init_global().expect("初始化全局配置失败");

    let config = ConfigLoader::get_global().expect("全局配置单例未初始化");

    // 初始化日志和链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "friend-service")?;
        info!(
            "链路追踪功能已启用，追踪数据将发送到: {}",
            config.telemetry.endpoint
        );
    } else {
        // 只初始化日志系统
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }

    info!("正在启动好友服务...");

    // 使用已加载的配置
    let host = &config.rpc.friend.host;
    let port = config.rpc.friend.port;
    let addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;

    // 初始化数据库连接池
    let db_pool = match PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database.pg_url())
        .await
    {
        Ok(pool) => {
            info!("数据库连接成功");
            pool
        }
        Err(err) => {
            error!("数据库连接失败: {}", err);
            return Err(err.into());
        }
    };

    // 初始化好友服务
    let friend_service = FriendServiceImpl::new(db_pool);

    // 创建并注册到服务注册中心
    let service_id = register_service(&config, Component::FriendServer).await?;

    info!("好友服务已注册到服务注册中心, 服务ID: {}", service_id);

    // 设置关闭通道
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let config_clone = config.clone();
    let service_id_clone = service_id.clone();
    let shutdown_signal_task = tokio::spawn(async move {
        common::service::shutdown_signal(shutdown_tx, service_id_clone, &config_clone).await
    });

    // 创建反射服务
    let reflection_service = ReflectionBuilder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build()?;

    // 创建日志拦截器
    let logging_interceptor = LoggingInterceptor::new();

    // 启动gRPC服务
    info!("好友服务启动，监听地址: {}", addr);

    // 创建服务器并运行
    let server = Server::builder()
        .add_service(reflection_service) // 添加反射服务
        .add_service(FriendServiceServer::with_interceptor(
            friend_service,
            logging_interceptor,
        ))
        .serve_with_shutdown(addr, async {
            let _ = shutdown_rx.await;
            info!("接收到关闭信号，gRPC服务准备关闭");
        });

    // 等待服务器关闭
    server.await?;
    info!("gRPC服务已关闭");

    // 等待关闭信号处理完成
    let _ = shutdown_signal_task.await?;

    // 在程序结束前关闭链路追踪，确保所有数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪...");
        common::logging::shutdown_telemetry();
    }

    info!("好友服务已完全关闭");
    Ok(())
}
