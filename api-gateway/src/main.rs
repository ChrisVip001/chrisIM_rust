use axum::{http::HeaderValue, Router};
use axum_server::Handle;
use std::net::SocketAddr;
use std::time::Duration;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::{error, info};

use runtime::config::AppConfig;
use std::sync::Arc;
use cache::SessionCache;
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
    runtime::service::init_rustls();

    // 加载配置
    let config_path = std::env::var("CONFIG_PATH")
        .unwrap_or_else(|_| "./config/config.yaml".to_string());

    
    let app_config = AppConfig::from_file(Some(&config_path))?;
    // 先用同一份配置构造局部 Arc，main 里不再回读全局单例
    let config = std::sync::Arc::new(app_config.clone());

    // 初始化日志和链路追踪
    if config.telemetry.enabled {
        runtime::logging::init_telemetry(&config, "api-gateway")?;
        info!("链路追踪已启用: {}", config.telemetry.endpoint);
    } else {
        runtime::logging::init_from_config(&config, "api-gateway")?;
        info!("仅启用日志系统");
    }

    info!("正在启动API网关服务...");

    // 初始化IP地理位置服务
    if let Err(e) = init_ip_location((&config.database.xdb).as_ref()) {
        error!("IP地理位置服务初始化失败: {}", e);
    }

    // 初始化缓存
    let cache_instance: Arc<cache::RedisCache> = cache::cache(&config).await;
    info!("缓存服务初始化完成");

    // 初始化指标系统
    metrics::init_metrics();

    // 构建应用
    let app = build_app(config.clone(), cache_instance).await?;

    // 启动服务器
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("API网关监听: http://{}:{}", config.server.host, config.server.port);

    // 启动服务器
    let handle = Handle::new();

    // 优雅停机：信号到达后调用 handle.graceful_shutdown，
    // 服务器停止接受新连接，并等待存量请求排空（最多 30s）。
    // 此前信号任务发出的 oneshot 无人消费，handle 从未被调用，
    // 进程退出时会硬断所有在途请求。
    let shutdown_handle = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        runtime::shutdown::wait_for_shutdown_signal().await;
        info!("准备优雅关闭API网关（存量请求最多等待30秒）...");
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(30)));
    });

    if let Err(err) = axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
    {
        error!("服务器错误: {}", err);
    }

    // 等待优雅停机任务收尾
    shutdown_task.await?;
    runtime::logging::shutdown_telemetry();
    info!("API网关服务已关闭");

    Ok(())
}

/// 构建应用
async fn build_app(config: std::sync::Arc<AppConfig>, cache_instance: std::sync::Arc<cache::RedisCache>) -> anyhow::Result<Router> {
    // 创建服务代理
    let service_proxy = proxy::ServiceProxy::new(config.clone()).await;
    
    // 构建路由
    let router = router::build_routes(service_proxy, config.clone(), cache_instance).await?;
    
    // 配置中间件栈
    // 限流/熔断从 gateway 配置读取并挂载到全局中间件栈
    let rate_limit_layer = rate_limit::RateLimitLayer::new(&config.gateway.rate_limit);
    Ok(router
        .layer(TraceLayer::new_for_http())
        .layer(middleware::RequestLoggerLayer)
        .layer(metrics::MetricsLayer)
        .layer(circuit_breaker::CircuitBreakerLayer::new(config.clone()))
        .layer(rate_limit_layer)
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
            // 添加自定义的system-type请求头
            axum::http::HeaderName::from_static("system-type"),
        ])
        .allow_credentials(true)
        .max_age(Duration::from_secs(3600))
}
