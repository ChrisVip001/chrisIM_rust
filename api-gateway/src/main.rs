use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use axum::{
    Router,
    http::StatusCode,
    response::IntoResponse,
};
use axum_server::{self, Handle};
use clap::Parser;
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tokio::signal;
// 直接使用tracing宏
use tracing::{info, error};

mod config;
mod auth;
mod rate_limit;
mod circuit_breaker;
mod metrics;
#[path = "tracing/mod.rs"]
mod tracing_setup;
pub mod proxy;
mod router;

use config::CONFIG;
pub use proxy::grpc_clients::user_client::UserServiceGrpcClient;

#[derive(Parser, Debug)]
#[clap(name = "api-gateway", about = "API网关服务")]
struct Args {
    /// 配置文件路径
    #[clap(short, long, default_value = ".env")]
    config: String,
    
    /// 配置文件路径
    #[clap(short = 'c', long, default_value = "config/gateway.yaml")]
    config_file: String,
    
    /// 监听地址
    #[clap(short, long)]
    host: Option<String>,
    
    /// 监听端口
    #[clap(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化命令行参数
    let args = Args::parse();
    
    // 加载.env文件
    dotenv::from_path(&args.config).ok();
    
    // 初始化日志和链路追踪
    if let Err(e) = tracing_setup::init_tracer().await {
        eprintln!("警告: 无法初始化链路追踪: {}", e);
    }
    
    info!("正在启动API网关服务...");
    
    // 加载配置
    config::load_config(&args.config_file).await?;
    
    // 获取服务地址和端口
    let _config = CONFIG.read().await;
    let host = args.host.unwrap_or_else(|| 
        std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
    );
    let port = args.port.unwrap_or_else(|| 
        std::env::var("GATEWAY_PORT")
            .unwrap_or_else(|_| "8000".to_string())
            .parse::<u16>()
            .unwrap_or(8000)
    );
    
    // 初始化Prometheus指标
    metrics::init_metrics();
    
    // 初始化服务代理
    let service_proxy = proxy::ServiceProxy::new().await;
    
    // 创建路由器
    let router_builder = router::RouterBuilder::new(Arc::from(service_proxy.clone()));
    let router = router_builder.build().await?;
    
    // 配置中间件
    let app = configure_middleware(router, service_proxy.clone()).await;
    
    // 绑定地址
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("API网关服务监听: http://{}:{}", host, port);
    
    // 创建服务器句柄
    let handle = Handle::new();
    
    // 创建优雅关闭任务
    let shutdown_handle = handle.clone();
    let service_proxy_clone = service_proxy.clone();
    tokio::spawn(async move {
        shutdown_signal(shutdown_handle, service_proxy_clone).await;
    });
    
    // 启动服务
    if let Err(err) = axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service())
        .await
    {
        error!("服务器错误: {}", err);
    }
    
    info!("API网关服务已关闭");
    Ok(())
}

/// 健康检查处理函数
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// 配置中间件
async fn configure_middleware(app: Router, _service_proxy: proxy::ServiceProxy) -> Router {
    // 添加链路追踪中间件
    let app = app.layer(TraceLayer::new_for_http());
    
    // 添加指标中间件
    let app = app.layer(metrics::MetricsLayer);
    
    // 添加CORS中间件
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_credentials(true);
    
    // 添加请求体大小限制和超时
    app.layer(cors)
       .layer(TimeoutLayer::new(Duration::from_secs(30)))
       .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
}

/// 优雅关闭信号处理
async fn shutdown_signal(handle: Handle, service_proxy: proxy::ServiceProxy) {
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
    
    // 清理资源
    service_proxy.shutdown().await;
    
    // 发送优雅关闭信号，设置30秒超时
    handle.graceful_shutdown(Some(Duration::from_secs(30)));
    
    info!("服务关闭准备完成");
} 