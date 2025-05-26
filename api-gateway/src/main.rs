use axum::{http::HeaderValue, Router};
use axum_server::Handle;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::oneshot;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::{error, info};

use common::{
    config::{AppConfig, Component, ConfigLoader},
    grpc_client::base::register_service,
};
use crate::api_utils::ip_region::ip_location::init_ip_location;

mod api_utils;
mod auth;
mod circuit_breaker;
mod metrics;
mod middleware;
mod proxy;
mod rate_limit;
mod router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化加密提供程序
    common::service::init_rustls();

    // 加载配置
    let config_path = std::env::var("CONFIG_PATH")
        .unwrap_or_else(|_| "./config/config.yaml".to_string());
    
    let app_config = AppConfig::from_file(Some(&config_path))?;
    ConfigLoader::set_global(app_config);
    let config = ConfigLoader::get_global().expect("未找到配置文件");

    // 初始化日志和链路追踪
    if config.telemetry.enabled {
        common::logging::init_telemetry(&config, "api-gateway")?;
        info!("链路追踪已启用: {}", config.telemetry.endpoint);
    } else {
        common::logging::init_from_config(&config)?;
        info!("仅启用日志系统");
    }

    info!("正在启动API网关服务...");

    // 初始化IP地理位置服务
    if let Err(e) = init_ip_location((&config.database.xdb).as_ref()) {
        error!("IP地理位置服务初始化失败: {}", e);
    }

    // 初始化指标系统
    metrics::init_metrics();

    // 构建应用
    let app = build_app(&config).await?;

    // 启动服务器
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("API网关监听: https://{}:{}", config.server.host, config.server.port);

    // 注册服务
    let service_id = register_service(&config, Component::ApiGateway).await?;
    info!("API网关已就绪, 服务ID: {}", service_id);

    // 启动服务器
    let handle = Handle::new();
    let (shutdown_tx, _shutdown_rx) = oneshot::channel::<()>();
    
    let config_clone = config.clone();
    let service_id_clone = service_id.clone();
    let shutdown_task = tokio::spawn(async move {
        common::service::shutdown_signal(shutdown_tx, service_id_clone, &config_clone).await
    });

    if let Err(err) = axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
    {
        error!("服务器错误: {}", err);
    }

    shutdown_task.await??;
    common::logging::shutdown_telemetry();
    info!("API网关服务已关闭");

    Ok(())
}

/// 构建应用
async fn build_app(config: &AppConfig) -> anyhow::Result<Router> {
    // 创建服务代理
    let service_proxy = proxy::ServiceProxy::new().await;
    
    // 构建路由
    let router = router::build_routes(service_proxy, &config.gateway).await?;
    
    // 配置中间件栈
    Ok(router
        .layer(TraceLayer::new_for_http())
        .layer(middleware::RequestLoggerLayer)
        .layer(metrics::MetricsLayer)
        .layer(build_cors_layer())
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)))
}

/// 构建CORS层
fn build_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin([
            "http://localhost:3000".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:3000".parse::<HeaderValue>().unwrap(),
            "http://localhost:5173".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5173".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
            axum::http::Method::PATCH,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::ACCEPT,
            axum::http::header::ORIGIN,
            axum::http::header::USER_AGENT,
        ])
        .allow_credentials(true)
        .max_age(Duration::from_secs(3600))
}
