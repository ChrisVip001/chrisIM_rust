use axum::http::HeaderValue;
use axum::{
    http::{self, Method},
    Router,
};
use axum_server::{self, Handle};
use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
// 直接使用tracing宏
use tracing::{error, info, warn};

mod auth;
mod circuit_breaker;
mod config;
mod metrics;
mod middleware;
pub mod proxy;
mod rate_limit;
mod router;
#[path = "tracing/mod.rs"]
mod tracing_setup;
mod api_doc;

pub use common::grpc_client::user_client::UserServiceGrpcClient;
pub use common::grpc_client::friend_client::FriendServiceGrpcClient;
pub use common::grpc_client::group_client::GroupServiceGrpcClient;
use common::service_registry::ServiceRegistry;
use config::CONFIG;

#[derive(Parser, Debug)]
#[clap(name = "api-gateway", about = "API网关服务")]
struct Args {
    /// 配置文件路径
    #[clap(short = 'f', long, default_value = "config/gateway.yaml")]
    config_file: String,

    /// 监听地址
    #[clap(short = 'H', long)]
    host: Option<String>,

    /// 监听端口
    #[clap(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化命令行参数
    let args = Args::parse();

    // 加载配置
    config::load_config(&args.config_file).await?;

    // 初始化日志和链路追踪
    if let Err(e) = tracing_setup::init_tracer().await {
        eprintln!("警告: 无法初始化链路追踪: {}", e);
    }

    info!("正在启动API网关服务...");

    // 获取服务地址和端口
    let _ = CONFIG.read().await;
    let host = args.host.unwrap_or_else(|| {
        std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
    });
    let port = args.port.unwrap_or_else(|| {
        std::env::var("GATEWAY_PORT")
            .unwrap_or_else(|_| "8000".to_string())
            .parse::<u16>()
            .unwrap_or(8000)
    });

    // 初始化Prometheus指标
    metrics::init_metrics();

    // 初始化服务代理
    let service_proxy = proxy::ServiceProxy::new().await;

    // 初始化 gRPC 客户端工厂
    proxy::GrpcClientFactoryImpl::new();
    info!("初始化 gRPC 客户端工厂完成，支持 HTTP 到 gRPC 的请求转发");

    // 创建路由器
    let router_builder = router::RouterBuilder::new(Arc::from(service_proxy.clone()));
    let router = router_builder.build().await?;

    // 配置中间件
    let app = configure_middleware(router, service_proxy.clone()).await;

    // 输出API服务信息
    info!("======================================================");
    info!("RustIM API服务启动");
    info!("======================================================");
    
    // 绑定地址
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("API网关服务监听: https://{}:{}", host, port);
    
    // 输出API文档地址
    info!("API文档可通过以下地址访问:");
    info!("- Swagger UI: https://{}:{}/swagger-ui", host, port);
    info!("- OpenAPI JSON: https://{}:{}/api-doc/openapi.json", host, port);
    info!("- 健康检查: https://{}:{}/health", host, port);
    info!("- API文档健康检查: https://{}:{}/api-doc/health", host, port);
    info!("======================================================");

    // 注册到 Consul
    let service_registry = ServiceRegistry::from_env();
    let service_id = service_registry
        .register_service(
            "api-gateway",
            &host,
            port as u32,
            vec!["api", "gateway"]
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
            "/health",
            "15s",
        )
        .await
        .unwrap_or_else(|e| {
            warn!("注册到 Consul 失败: {}, 服务发现功能可能不可用", e);
            "api-gateway-unregistered".to_string()
        });

    info!("API网关已注册到Consul, 服务ID: {}", service_id);

    // 创建服务器句柄
    let handle = Handle::new();

    // 创建优雅关闭任务
    let shutdown_handle = handle.clone();
    let service_proxy_clone = service_proxy.clone();
    let service_registry_clone = service_registry.clone();
    tokio::spawn(async move {
        shutdown_signal(shutdown_handle, service_proxy_clone, service_registry_clone).await;
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
    
    // 关闭链路追踪，确保所有数据都被发送
    info!("正在关闭链路追踪...");
    common::logging::shutdown_telemetry();
    
    Ok(())
}

/// 配置中间件
async fn configure_middleware(app: Router, _service_proxy: proxy::ServiceProxy) -> Router {
    // 创建用户服务客户端
    let service_client = common::grpc_client::GrpcServiceClient::from_env("user-service");
    let user_client = Arc::new(UserServiceGrpcClient::new(service_client));
    info!("创建并注册用户服务客户端扩展");

    // 添加链路追踪中间件
    let app = app.layer(TraceLayer::new_for_http());
    
    // 添加请求路径日志中间件
    let app = app.layer(middleware::RequestLoggerLayer);

    // 添加用户服务客户端扩展
    let app = app.layer(axum::Extension(user_client));

    // 添加指标中间件
    let app = app.layer(metrics::MetricsLayer);

    // 添加CORS中间件
    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:3000".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:3000".parse::<HeaderValue>().unwrap(),
            "http://localhost:5173".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5173".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
            Method::PATCH,
        ])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
            http::header::ACCEPT,
            http::header::ORIGIN,
            http::header::USER_AGENT,
        ])
        .allow_credentials(true)
        .max_age(Duration::from_secs(3600));

    // 添加请求体大小限制和超时
    app.layer(cors)
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
}

/// 优雅关闭信号处理
async fn shutdown_signal(
    handle: Handle,
    service_proxy: proxy::ServiceProxy,
    service_registry: ServiceRegistry,
) {
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

    // 从 Consul 注销服务
    match service_registry.deregister_service().await {
        Ok(_) => info!("已从Consul注销服务"),
        Err(e) => error!("从Consul注销服务失败: {}", e),
    }

    // 清理资源
    service_proxy.shutdown().await;

    // 发送优雅关闭信号，设置30秒超时
    handle.graceful_shutdown(Some(Duration::from_secs(30)));

    info!("服务关闭准备完成");
}
