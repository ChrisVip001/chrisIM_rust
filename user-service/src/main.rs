use anyhow::Result;
use common::config::AppConfig;
use common::service_registry::ServiceRegistry;
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tonic::transport::Server;
use tracing::{info, warn, error, Level};
use tracing_subscriber::FmtSubscriber;
use tokio::signal;
use tokio::sync::oneshot;
use axum::{Router, routing::get};
use axum_server;

mod model;
mod repository;
mod service;

use service::user_service::UserServiceImpl;
use common::proto::user::user_service_server::UserServiceServer;

#[derive(Parser, Debug)]
#[clap(name = "user-service", about = "用户服务")]
struct Args {
    /// 配置文件路径
    #[clap(short, long, default_value = ".env")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化命令行参数
    let args = Args::parse();
    
    // 加载.env文件
    dotenv::from_path(&args.config).ok();
    
    // 初始化日志
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    info!("正在启动用户服务...");
    
    // 加载配置
    let config = AppConfig::new()?;
    let host = &config.server.host;
    let port = config.server.port;
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
    
    // 初始化用户服务
    let user_service = UserServiceImpl::new(db_pool);
    
    // 创建HTTP服务器用于健康检查
    let health_port = port + 1;
    let health_service = start_health_service(host, health_port).await?;
    
    // 创建并注册到Consul
    let service_registry = ServiceRegistry::from_env();
    let service_id = service_registry.register_service(
        "user-service",
        host,
        health_port as u32, // 显式转换为u32类型
        vec!["user".to_string(), "api".to_string()],
        "/health",
        "15s",
    ).await?;
    
    info!("用户服务已注册到Consul, 服务ID: {}", service_id);
    
    // 设置关闭通道
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_signal_task = tokio::spawn(shutdown_signal(shutdown_tx, service_registry.clone()));
    
    // 启动gRPC服务
    info!("用户服务启动，监听地址: {}", addr);
    
    // 创建服务器并运行
    let server = Server::builder()
        .add_service(UserServiceServer::new(user_service))
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
    
    info!("用户服务已完全关闭");
    Ok(())
}

// 健康检查HTTP服务
async fn start_health_service(host: &str, port: u16) -> Result<impl std::future::Future<Output = ()>> {
    let health_addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;
    
    // 创建HTTP服务
    let app = Router::new()
        .route("/health", get(health_check));
    
    info!("健康检查服务启动，监听地址: {}", health_addr);
    
    // 启动HTTP服务
    let health_server = axum_server::bind(health_addr)
        .serve(app.into_make_service());
    
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
        signal::ctrl_c()
            .await
            .expect("无法安装Ctrl+C处理器");
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