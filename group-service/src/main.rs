use anyhow::Result;
use axum::{routing::get, Router};
use axum_server;
use clap::Parser;
use common::config::AppConfig;
use common::grpc::LoggingInterceptor;
use common::service_registry::ServiceRegistry;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::signal;
use tokio::sync::oneshot;
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info, warn};

mod model;
mod repository;
mod service;

use common::proto::group::group_service_server::GroupServiceServer;
use service::group_service::GroupServiceImpl;
// 导入群组服务proto文件描述符，用于gRPC反射
const FILE_DESCRIPTOR_SET: &[u8] = common::proto::group::FILE_DESCRIPTOR_SET;

#[derive(Parser, Debug)]
#[clap(name = "group-service", about = "群组服务")]
struct Args {
    /// 配置文件路径
    #[clap(short, long, default_value = "config/config.yaml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化命令行参数
    let args = Args::parse();

    // 加载配置
    let config = AppConfig::from_file(Some(&args.config))?;

    // 初始化日志和链路追踪
    // 根据配置判断是否启用链路追踪
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
    let host = &config.server.host;
    let port = 50003; // 指定群组服务端口
    let addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;

    // 初始化数据库连接池
    let db_pool = match PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database.url())
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
    let group_service = GroupServiceImpl::new(db_pool);

    // 创建HTTP服务器用于健康检查
    let health_port = port + 1;
    let health_check_url = format!("http://{}:{}/health", host, health_port);
    let health_service = start_health_service(host, health_port).await?;

    // 创建并注册到Consul
    let service_registry = ServiceRegistry::from_env();
    let service_id = service_registry
        .register_service(
            "group-service",
            host,
            port as u32, // 注册gRPC服务端口
            vec!["group".to_string(), "api".to_string()],
            &health_check_url, // 明确指定健康检查URL
            "15s",
        )
        .await?;
    
    info!("群组服务已注册到Consul, 服务ID: {}", service_id);

    // 设置关闭通道
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_signal_task = tokio::spawn(shutdown_signal(shutdown_tx, service_registry.clone()));
    // 创建反射服务
    let reflection_service = ReflectionBuilder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build()?;

    // 创建日志拦截器
    let logging_interceptor = LoggingInterceptor::new();

    // 启动gRPC服务
    info!("群组服务启动，监听地址: {}", addr);

    // 创建服务器并运行
    let server = Server::builder()
        .add_service(reflection_service) // 添加反射服务
        .add_service(GroupServiceServer::with_interceptor(
            group_service, 
            logging_interceptor
        ))
        .serve_with_shutdown(addr, async {
            let _ = shutdown_rx.await;
            info!("接收到关闭信号，gRPC服务准备关闭");
        });

    tokio::select! {
        _ = server => {
            info!("gRPC服务已关闭");
        }
        _ = health_service => {
            info!("健康检查服务已关闭");
        }
    }

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

// 健康检查HTTP服务
async fn start_health_service(
    host: &str,
    port: u16,
) -> Result<impl std::future::Future<Output = ()>> {
    let health_addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;

    // 创建HTTP服务
    let app = Router::new().route("/health", get(health_check));

    info!("健康检查服务启动，监听地址: {}", health_addr);

    // 启动HTTP服务
    let health_server = axum_server::bind(health_addr).serve(app.into_make_service());

    let server_task = tokio::spawn(async move {
        if let Err(e) = health_server.await {
            error!("健康检查服务错误: {}", e);
        }
    });

    Ok(async move {
        server_task.await.unwrap();
    })
}

// 健康检查端点
async fn health_check() -> &'static str {
    "OK"
}

// 优雅关闭信号处理
async fn shutdown_signal(tx: oneshot::Sender<()>, service_registry: ServiceRegistry) -> Result<()> {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("无法安装Ctrl+C处理器");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法安装SIGTERM处理器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("接收到关闭信号，准备优雅关闭...");

    // 从Consul注销服务
    match service_registry.deregister_service().await {
        Ok(_) => info!("已从Consul注销服务"),
        Err(e) => error!("从Consul注销服务失败: {}", e),
    }

    // 发送关闭信号
    if let Err(_) = tx.send(()) {
        warn!("无法发送关闭信号，接收端可能已关闭");
    }

    info!("服务关闭准备完成");
    Ok(())
}
