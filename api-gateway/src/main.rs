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
    config::{AppConfig, ConfigLoader},
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
        common::logging::init_from_config(&config, "api-gateway")?;
        info!("仅启用日志系统");
    }

    info!("正在启动API网关服务...");

    // 初始化IP地理位置服务
    if let Err(e) = init_ip_location((&config.database.xdb).as_ref()) {
        error!("IP地理位置服务初始化失败: {}", e);
    }

    // 初始化缓存
    let cache_instance = cache::cache(&config).await;
    info!("缓存服务初始化完成");

    // 初始化指标系统
    metrics::init_metrics();

    // 构建应用
    let app = build_app(&config, cache_instance).await?;

    // 启动服务器
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("API网关监听: https://{}:{}", config.server.host, config.server.port);

    // 启动服务器
    let handle = Handle::new();
    let (shutdown_tx, _shutdown_rx) = oneshot::channel::<()>();
    
    let shutdown_task = tokio::spawn(async move {
        // api-gateway不需要从服务注册中心注销，直接监听关闭信号
        use tokio::signal;
        
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

        info!("接收到关闭信号，准备优雅关闭API网关...");
        
        if let Err(_) = shutdown_tx.send(()) {
            tracing::warn!("无法发送关闭信号，接收端可能已关闭");
        }
        
        Ok::<(), anyhow::Error>(())
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
async fn build_app(config: &AppConfig, cache_instance: std::sync::Arc<dyn cache::Cache>) -> anyhow::Result<Router> {
    // 创建服务代理
    let service_proxy = proxy::ServiceProxy::new().await;
    
    // 构建路由
    let router = router::build_routes(service_proxy, &config.gateway, cache_instance).await?;
    
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
            // 添加更多开发端口支持
            "http://localhost:63342".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:63342".parse::<HeaderValue>().unwrap(),
            "http://localhost:8080".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:8080".parse::<HeaderValue>().unwrap(),
            "http://localhost:8081".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:8081".parse::<HeaderValue>().unwrap(),
            // 常见的开发端口
            "http://localhost:3001".parse::<HeaderValue>().unwrap(),
            "http://localhost:3002".parse::<HeaderValue>().unwrap(),
            "http://localhost:8000".parse::<HeaderValue>().unwrap(),
            "http://localhost:9000".parse::<HeaderValue>().unwrap(),
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
            // 添加更多常用的请求头
            axum::http::header::CACHE_CONTROL,
            axum::http::header::PRAGMA,
        ])
        .allow_credentials(true)
        .max_age(Duration::from_secs(3600))
}
