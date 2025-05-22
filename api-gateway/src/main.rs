use axum::http::HeaderValue;
use axum::{
    http::{self, Method},
    Router,
};
use axum_server::{self, Handle};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
// 直接使用tracing宏
use common::config::{Component, ConfigLoader};
use common::grpc_client::base::register_service;
use tracing::{error, info};

mod api_doc;
mod api_utils;
mod auth;
mod circuit_breaker;
mod metrics;
mod middleware;
pub mod proxy;
mod rate_limit;
mod router;

pub use common::grpc_client::friend_client::FriendServiceGrpcClient;
pub use common::grpc_client::group_client::GroupServiceGrpcClient;
pub use common::grpc_client::user_client::UserServiceGrpcClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化rustls加密提供程序
    common::service::init_rustls();

    // 加载配置
    info!("初始化全局配置单例");
    ConfigLoader::init_global()?;

    let config = ConfigLoader::get_global().expect("全局配置单例未初始化");

    // 初始化日志和链路追踪
    // 根据配置判断是否启用链路追踪
    if config.telemetry.enabled {
        // 启动带有分布式链路追踪的日志系统
        common::logging::init_telemetry(&config, "api-gateway")?;
        info!(
            "链路追踪功能已启用，追踪数据将发送到: {}",
            config.telemetry.endpoint
        );
    } else {
        // 只初始化日志系统
        common::logging::init_from_config(&config)?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }

    info!("正在启动API网关服务...");

    // 初始化Prometheus指标
    metrics::init_metrics();

    // 初始化服务代理
    let service_proxy = proxy::ServiceProxy::new().await;

    // 初始化 gRPC 客户端工厂
    proxy::GrpcClientFactoryImpl::new();
    info!("初始化 gRPC 客户端工厂完成，支持 HTTP 到 gRPC 的请求转发");

    // 创建路由器
    let router_builder = router::RouterBuilder::new(Arc::from(service_proxy.clone()));
    let router = router_builder
        .build(Arc::new(config.gateway.clone()))
        .await?;

    // 配置中间件
    let app = configure_middleware(router).await;

    // 输出API服务信息
    info!("======================================================");
    info!("RustIM API服务启动");
    info!("======================================================");

    let (host, port) = (config.server.host.clone(), config.server.port);
    // 绑定地址
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("API网关服务监听: https://{}:{}", host, port);

    // 注册到 Consul
    let service_id = register_service(&config, Component::ApiGateway).await?;

    info!("API网关已准备就绪, 服务ID: {}", service_id);

    // 创建服务器句柄
    let handle = Handle::new();

    // 设置关闭通道
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let config_clone = config.clone();
    let service_id_clone = service_id.clone();
    let shutdown_signal_task = tokio::spawn(async move {
        common::service::shutdown_signal(shutdown_tx, service_id_clone, &config_clone).await
    });

    // 启动服务
    if let Err(err) = axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service())
        .await
    {
        let _ = shutdown_rx.await;
        error!("服务器错误: {}", err);
    }

    // 等待关闭信号处理完成
    let _ = shutdown_signal_task.await?;
    // 关闭链路追踪，确保所有数据都被发送
    info!("正在关闭链路追踪...");

    common::logging::shutdown_telemetry();
    info!("API网关服务已关闭");

    Ok(())
}

/// 配置中间件
async fn configure_middleware(app: Router) -> Router {
    // 添加链路追踪中间件
    let app = app.layer(TraceLayer::new_for_http());

    // 添加请求路径日志中间件
    let app = app.layer(middleware::RequestLoggerLayer);

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
