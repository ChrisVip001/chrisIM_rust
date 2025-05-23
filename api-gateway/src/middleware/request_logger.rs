use std::task::{Context, Poll};
use std::time::Instant;

use axum::http;
use futures::future::BoxFuture;
use tower::{Layer, Service};
use tracing::{info, warn};

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
        
        let start_time = Instant::now();

        // 记录请求开始
        info!(
            method = %method,
            path = %path,
            request_id = %request_id,
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
                        "HTTP请求处理失败"
                    );

                    Err(err)
                }
            }
        })
    }
}