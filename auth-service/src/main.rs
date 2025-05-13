use anyhow::Result;
use common::config::DynamicConfig;
use common::service_registry::ServiceRegistry;
use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::Server;
use tracing::{info, warn, error, Level};
use tracing_subscriber::FmtSubscriber;
use tokio::signal;
use tokio::sync::oneshot;
use axum::{Router, routing::get};
use axum_server;

mod service;
mod repository;

use service::auth_service::AuthServiceImpl;
use common::proto::auth::auth_service_server::AuthServiceServer;

#[derive(Parser, Debug)]
#[clap(name = "auth-service", about = "认证服务")]
struct Args {
    /// 配置文件路径
    #[clap(short, long)]
    config: Option<String>,
    
    /// 配置刷新间隔（秒）
    #[clap(short, long, default_value = "60")]
    refresh: u64,
    
    /// 是否使用Kubernetes ConfigMap
    #[clap(long)]
    k8s_config: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化命令行参数
    let args = Args::parse();
    
    // 初始化日志
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    // 配置文件路径
    let mut config_paths = Vec::new();
    
    // 如果指定了配置文件，添加到路径列表
    if let Some(config_file) = &args.config {
        config_paths.push(config_file.clone());
    }
    
    // 如果使用Kubernetes ConfigMap，添加ConfigMap挂载路径
    if args.k8s_config {
        config_paths.push("/config/config.yaml".to_string());
        config_paths.push("/config/config.json".to_string());
        config_paths.push("/config/config.toml".to_string());
        config_paths.push("/config/.env".to_string());
    }
    
    // 添加默认配置路径
    config_paths.push("config.yaml".to_string());
    config_paths.push("config.json".to_string());
    config_paths.push("config.toml".to_string());
    config_paths.push(".env".to_string());
    
    // 创建动态配置
    let dynamic_config = Arc::new(DynamicConfig::new(
        config_paths, 
        args.refresh
    )?);
    
    // 启动配置监控线程
    dynamic_config.clone().start_refresh_task();
    
    // 获取初始配置
    let config = dynamic_config.get_config();
    let host = &config.server.host;
    let port = config.server.port;
    let addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;
    
    // 初始化Redis连接池
    let redis_client = redis::Client::open(config.redis.url())?;
    let redis_conn = redis_client.get_multiplexed_async_connection().await?;
    
    // 初始化认证服务
    let auth_service = AuthServiceImpl::new(
        (*config).clone(),
        redis_conn,
    );
    
    // 创建HTTP服务器用于健康检查
    let health_port = port + 1;
    let health_service = start_health_service(host, health_port).await?;
    
    // 创建并注册到Consul
    let service_registry = ServiceRegistry::from_env();
    let service_id = service_registry.register_service(
        "auth-service",
        host,
        health_port as u32, // 显式转换为u32类型
        vec!["auth".to_string(), "api".to_string()],
        "/health",
        "15s",
    ).await?;
    
    info!("认证服务已注册到Consul, 服务ID: {}", service_id);
    
    // 设置关闭通道
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_signal_task = tokio::spawn(shutdown_signal(shutdown_tx, service_registry.clone()));
    
    // 启动gRPC服务
    info!("认证服务启动，监听地址: {}", addr);
    
    // 创建服务器并运行
    let server = Server::builder()
        .add_service(AuthServiceServer::new(auth_service))
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
    
    info!("认证服务已完全关闭");
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