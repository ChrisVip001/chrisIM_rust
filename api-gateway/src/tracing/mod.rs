use tracing_subscriber::{layer::SubscriberExt, EnvFilter};
use tracing_subscriber::fmt::Layer as FmtLayer;
use tracing_subscriber::util::SubscriberInitExt;
use axum::{
    http::{Request, HeaderMap},
    body::Body,
    middleware::Next,
    response::Response,
};
use tracing::{info, info_span};
use crate::config::CONFIG;

/// 初始化链路追踪
pub async fn init_tracer() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 读取配置
    let config = CONFIG.read().await;
    
    // 如果未启用OpenTelemetry，只设置标准日志
    if !config.tracing.enable_opentelemetry {
        let fmt_layer = FmtLayer::new();
        
        tracing_subscriber::registry()
            .with(EnvFilter::from_default_env())
            .with(fmt_layer)
            .init();
        
        info!("已初始化日志系统，未启用OpenTelemetry链路追踪");
        return Ok(());
    }
    
    // 如果启用OpenTelemetry，我们在这里简化实现
    // 由于版本兼容性问题，我们暂时只使用标准日志
    info!("由于OpenTelemetry版本兼容性问题，暂时只使用标准日志");
    
    // 使用标准格式输出
    let fmt_layer = FmtLayer::new();
    
    // 初始化订阅者
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt_layer)
        .init();
    
    info!("已初始化日志系统");
    
    Ok(())
}

/// 链路追踪中间件
pub async fn trace_middleware(req: Request<Body>, next: Next) -> Response {
    // 创建请求跟踪span
    let path = req.uri().path().to_string();
    let method = req.method().as_str().to_string();
    let span = info_span!(
        "http_request",
        path = %path,
        method = %method,
        http.target = %req.uri().path(),
        http.host = ?req.uri().host(),
        http.user_agent = ?req.headers().get("user-agent").and_then(|v| v.to_str().ok()),
    );
    
    // 在span上下文中执行请求
    let _enter = span.enter();
    
    // 继续执行请求
    let response = next.run(req).await;
    
    // 记录响应状态码
    span.record("http.status_code", &response.status().as_u16());
    
    response
}

/// 从请求头中提取跟踪上下文
fn extract_trace_context(headers: &HeaderMap) -> Option<(String, String)> {
    let traceparent = headers.get("traceparent").and_then(|v| v.to_str().ok())?;
    
    // 解析traceparent头 (格式: 00-<trace-id>-<parent-id>-<trace-flags>)
    let parts: Vec<&str> = traceparent.split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    
    Some((parts[1].to_string(), parts[2].to_string()))
} 