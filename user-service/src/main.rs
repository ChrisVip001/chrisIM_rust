use anyhow::Result;
use std::env;
use common::config::{AppConfig, Component, ConfigLoader};
use common::grpc::LoggingInterceptor;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tonic::codegen::Body;
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info};
// 添加gRPC健康检查相关导入
use tonic_health::server::HealthReporter;

mod model;
mod repository;
mod service;

use common::proto::user::user_service_server::UserServiceServer;
use common::service::shutdown_signal;
use service::user_service::UserServiceImpl;

// 导入用户服务proto文件描述符，用于gRPC反射
const FILE_DESCRIPTOR_SET: &[u8] = common::proto::user::FILE_DESCRIPTOR_SET;

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
    common::service::init();

    // 初始化日志和链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "user-service")?;
        info!(
            "链路追踪功能已启用，追踪数据将发送到: {}",
            config.telemetry.endpoint
        );
    } else {
        // 只初始化日志系统
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }

    info!("正在启动用户服务...");

    // 使用已加载的配置
    let host = &config.rpc.user.host;
    let port = config.rpc.user.port;
    let addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;

    // 初始化数据库连接池
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database.pg_url())
        .await
        .map_err( |e| anyhow::anyhow!("数据库连接失败:{}", e))?;

    // 初始化用户服务
    let user_service = UserServiceImpl::new(db_pool.clone());

    // 设置关闭通道
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_signal_task =
        tokio::spawn(
            async move { shutdown_signal(shutdown_tx, Component::UserServer).await },
        );

    // 创建反射服务
    let reflection_service = ReflectionBuilder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build()?;

    // 创建日志拦截器
    let logging_interceptor = LoggingInterceptor::new();

    // 创建gRPC健康检查服务
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    
    // 设置服务为健康状态 - 只要服务能启动就认为是健康的
    health_reporter
        .set_serving::<UserServiceServer<UserServiceImpl>>()
        .await;

    // 启动gRPC服务
    info!("用户服务启动，监听地址: {}", addr);
    common::service::register(Component::UserServer)
        .await?
        .map_err(|e| anyhow::anyhow!("服务注册失败:{}", e));

    // 创建服务器并运行
    let server = Server::builder()
        .add_service(health_service)     // 健康检查服务
        .add_service(reflection_service) // 反射服务
        .add_service(UserServiceServer::with_interceptor(user_service, logging_interceptor))
        .serve_with_shutdown(addr, async {
            shutdown_rx.await.ok();
        });

    // 等待服务器关闭
    server.await?;
    info!("gRPC服务已关闭");

    // 等待关闭信号处理完成
    shutdown_signal_task.await??;
    // 在程序结束前关闭链路追踪，确保所有数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪...");
        common::logging::shutdown_telemetry();
    }

    info!("用户服务已完全关闭");
    Ok(())
}
