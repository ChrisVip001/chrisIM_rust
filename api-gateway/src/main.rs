use axum::{http::{HeaderValue, StatusCode}, Router};
use axum_server::Handle;
use std::net::SocketAddr;
use std::time::Duration;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
};
use tracing::{error, info};

use runtime::config::AppConfig;
use std::sync::Arc;

mod api_utils;
mod auth;
mod circuit_breaker;
mod error;
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
    // 用同一份配置构造 Arc，后续各组件共享这一个实例
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

    // 初始化缓存
    let cache_instance: Arc<cache::RedisCache> = cache::cache(&config).await;
    info!("缓存服务初始化完成");

    // 初始化指标系统
    metrics::init_metrics();

    // 构建应用
    let app = build_app(config.clone(), cache_instance).await?;

    // 启动服务器：监听地址取自配置的 server.host + server.port
    let listen_host = config
        .server
        .host
        .parse::<std::net::IpAddr>()
        .ok()
        .filter(|ip| !ip.is_unspecified())
        .unwrap_or(std::net::IpAddr::from([0, 0, 0, 0]));
    let addr = SocketAddr::new(listen_host, config.server.port);
    info!("API网关监听: http://{}", addr);

    let handle = Handle::new();

    // 优雅停机：信号到达后调用 handle.graceful_shutdown，
    // 服务器停止接受新连接，并等待存量请求排空（最多 30s）。
    let shutdown_handle = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        runtime::shutdown::wait_for_shutdown_signal().await;
        info!("准备优雅关闭API网关（存量请求最多等待30秒）...");
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(30)));
    });

    // serve 返回 = 服务结束（正常停机或端口占用等错误）；
    // 出错时以非 0 退出码结束，交由编排层感知。
    let serve_result = axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await;

    // 只会命中「serve 提前失败」：正常情况下停机任务已经跑完
    if !shutdown_task.is_finished() {
        shutdown_task.abort();
    }

    runtime::logging::shutdown_telemetry();

    if let Err(err) = serve_result {
        error!("服务器错误: {}", err);
        return Err(err.into());
    }

    info!("API网关服务已关闭");
    Ok(())
}

/// 构建应用
async fn build_app(config: std::sync::Arc<AppConfig>, cache_instance: std::sync::Arc<cache::RedisCache>) -> anyhow::Result<Router> {
    // 创建服务代理
    let service_proxy = proxy::ServiceProxy::new(config.clone()).await;

    // 构建路由
    let router = router::build_routes(service_proxy, config.clone(), cache_instance).await?;

    // 中间件栈（`layer` 自下而上包裹：越靠后越外层）
    //
    // 顺序（外 → 内）：
    //   CORS → 体积限制 → 超时 → 访问日志 → 指标 → 限流 → 熔断 → 路由
    //
    // 日志与指标放在限流/超时之外，429、413、408 与熔断短路响应也会被记录。
    let trusted_proxies = config.gateway.auth.trusted_proxies.clone();
    let rate_limit_layer =
        rate_limit::RateLimitLayer::new(&config.gateway.rate_limit, trusted_proxies.clone());

    Ok(router
        .layer(circuit_breaker::CircuitBreakerLayer::new(config.clone()))
        .layer(rate_limit_layer)
        .layer(metrics::MetricsLayer)
        .layer(middleware::RequestLoggerLayer::new(trusted_proxies))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        .layer(build_cors_layer(&config)))
}

/// 构建CORS层
///
/// 来源由 `gateway.cors.allowed_origins` 决定；debug 构建额外放行本地开发端口，
/// release 构建不再把 IDE/开发端口带进生产。
fn build_cors_layer(config: &AppConfig) -> CorsLayer {
    let mut origins: Vec<HeaderValue> = config
        .gateway
        .cors
        .allowed_origins
        .iter()
        .filter_map(|o| match o.parse::<HeaderValue>() {
            Ok(v) => Some(v),
            Err(_) => {
                error!("忽略非法的 CORS 来源配置: {}", o);
                None
            }
        })
        .collect();

    if cfg!(debug_assertions) {
        for dev in [
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://localhost:8080",
            "http://127.0.0.1:8080",
        ] {
            if let Ok(v) = dev.parse::<HeaderValue>() {
                origins.push(v);
            }
        }
    }

    CorsLayer::new()
        .allow_origin(origins)
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
        .allow_credentials(config.gateway.cors.allow_credentials)
        .max_age(Duration::from_secs(3600))
}
