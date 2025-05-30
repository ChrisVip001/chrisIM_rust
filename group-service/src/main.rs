use anyhow::Result;
use std::env;
use common::config::{AppConfig, Component, ConfigLoader};
use common::grpc::LoggingInterceptor;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tonic::transport::Server;
use tonic_health::server::health_reporter;
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info};

mod model;
mod repository;
mod service;

use common::proto::group::group_service_server::GroupServiceServer;
use service::group_service::GroupServiceImpl;
// 导入群组服务proto文件描述符，用于gRPC反射
const FILE_DESCRIPTOR_SET: &[u8] = common::proto::group::FILE_DESCRIPTOR_SET;

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

    // 初始化统一服务模块
    common::service::init((*config).clone());

    // 初始化日志和链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "group-service")?;
        info!("链路追踪功能已启用，追踪数据将发送到: {}", config.telemetry.endpoint);
    } else {
        // 只初始化日志系统
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }

    info!("正在启动群组服务...");

    // 使用已加载的配置
    let host = &config.rpc.group.host;
    let port = config.rpc.group.port;
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

    // 初始化群组服务
    let group_service = GroupServiceImpl::new(db_pool.clone());

    // 注册服务到服务注册中心
    let service_id = common::service::register(Component::GroupServer).await?;

    info!(
        "群组服务已注册到服务注册中心, 服务ID: {}",
        service_id
    );

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

    // 创建gRPC健康检查服务
    let (mut health_reporter, health_service) = health_reporter();
    
    // 设置服务为健康状态 - 只要服务能启动就认为是健康的
    health_reporter
        .set_serving::<GroupServiceServer<GroupServiceImpl>>()
        .await;

    // 启动gRPC服务
    info!("群组服务启动，监听地址: {}", addr);

    // 创建服务器并运行
    let server = Server::builder()
        .add_service(health_service) // 添加健康检查服务
        .add_service(reflection_service) // 添加反射服务
        .add_service(GroupServiceServer::with_interceptor(
            group_service, 
            logging_interceptor
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

    info!("群组服务已完全关闭");
    Ok(())
}
