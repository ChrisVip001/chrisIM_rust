use std::task::{Context, Poll};
use std::time::Instant;

use axum::{
    extract::ConnectInfo,
    http,
};
use futures::future::BoxFuture;
use tower::{Layer, Service};
use tracing::{info, warn};
use std::net::SocketAddr;

use crate::api_utils::ip_region::ip_location;

/// 请求路径日志中间件
#[derive(Clone)]
pub struct RequestLoggerLayer;

impl<S> Layer<S> for RequestLoggerLayer {
    type Service = RequestLogger<S>;

    fn layer(&self, service: S) -> Self::Service {
        RequestLogger { inner: service }
    }
}

#[derive(Clone)]
pub struct RequestLogger<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for RequestLogger<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        // 克隆服务
        let mut inner = self.inner.clone();

        // 获取请求的信息 - 确保拥有所有数据而不是借用
        let method = req.method().clone();
        let path = req.uri().path().to_string(); // 使用to_string确保拥有数据
        
        // 提取请求标识符 - 确保拥有数据
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_string(); // 确保完全拥有

        // // 模拟ip试验查找ip属地功能
        // let analog_ip = req
        //     .headers()
        //     .get("analog_ip")
        //     .and_then(|v| v.to_str().ok())
        //     .unwrap_or("-")
        //     .to_string(); // 确保完全拥有
        // // 获取客户端IP
        // let mut client_ip = get_client_ip(&req);
        // if analog_ip !="-" {
        //     client_ip = analog_ip;
        // }

        // 获取客户端IP
        let client_ip = get_client_ip(&req);

        // 获取服务器IP（从请求的主机头或本地配置）
        let server_ip = get_server_ip(&req);
        
        // 获取客户端IP信息
        let ip_info = ip_location::get_ip_info(&client_ip);
        // 格式化IP地理位置信息
        let location_info = ip_location::format_ip_location(&ip_info);
        
        let start_time = Instant::now();

        // 记录请求开始
        info!(
            method = %method,
            path = %path,
            request_id = %request_id,
            client_ip = %client_ip,
            server_ip = %server_ip,
            ip_type = %format!("{:?}", ip_info.ip_type),
            location = %location_info,
            country = %ip_info.country,
            province = %ip_info.province,
            city = %ip_info.city,
            isp = %ip_info.isp,
            "收到HTTP请求"
        );

        // 在获取所有所需信息后，调用内部服务
        let future = inner.call(req);

        // 包装future实现日志记录
        Box::pin(async move {
            // 直接await内部future
            match future.await {
                Ok(response) => {
                    // 记录请求成功完成
                    let duration = start_time.elapsed();
                    let status = response.status().as_u16();

                    info!(
                        method = %method,
                        path = %path,
                        status = %status,
                        duration_ms = %duration.as_millis(),
                        request_id = %request_id,
                        client_ip = %client_ip,
                        server_ip = %server_ip,
                        location = %location_info,
                        "HTTP请求处理完成"
                    );

                    Ok(response)
                }
                Err(err) => {
                    // 记录请求失败，不使用Debug格式打印错误
                    let duration = start_time.elapsed();

                    warn!(
                        method = %method,
                        path = %path,
                        duration_ms = %duration.as_millis(),
                        request_id = %request_id,
                        client_ip = %client_ip,
                        server_ip = %server_ip,
                        location = %location_info,
                        "HTTP请求处理失败"
                    );

                    Err(err)
                }
            }
        })
    }
}

/// 从请求中获取客户端IP
fn get_client_ip<B>(request: &http::Request<B>) -> String {
    request
        .headers()
        .get("X-Forwarded-For")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or("").trim().to_string())
        .or_else(|| {
            request
                .headers()
                .get("X-Real-IP")
                .and_then(|value| value.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| {
            // 尝试从连接信息中获取
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connect_info| connect_info.0.ip().to_string())
                .unwrap_or_else(|| "未知客户端IP".to_string())
        })
}

/// 获取服务器IP
fn get_server_ip<B>(request: &http::Request<B>) -> String {
    // 首先尝试从Host头获取服务器地址
    request
        .headers()
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // 如果没有Host头，使用请求URI的authority部分
            request
                .uri()
                .authority()
                .map(|auth| auth.to_string())
                .unwrap_or_else(|| {
                    // 如果都没有，返回服务器的绑定地址（通常是0.0.0.0:端口号）
                    // 注意：这里我们无法直接获取实际绑定的地址，因为这不是请求级别的信息
                    // 所以我们返回一个静态字符串表示本地服务器
                    "服务器地址 (0.0.0.0:8080)".to_string()
                })
        })
}