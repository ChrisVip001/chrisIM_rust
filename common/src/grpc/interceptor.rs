use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::{Request, Status};
use tracing::info;

/// 用于记录gRPC请求的拦截器
#[derive(Debug, Clone, Default)]
pub struct LoggingInterceptor {}

impl LoggingInterceptor {
    pub fn new() -> Self {
        Self {}
    }
}

impl tonic::service::Interceptor for LoggingInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        // 获取请求的路径，通过metadata中的:path字段
        let path = request
            .metadata()
            .get(":path")
            .map(|v| v.to_str().unwrap_or("/unknown"))
            .unwrap_or("/unknown");
        
        // 从请求元数据中提取trace_id，如果存在
        let trace_id = request
            .metadata()
            .get("x-trace-id")
            .map(|v| v.to_str().unwrap_or("unknown"))
            .unwrap_or("none");
        
        // 提取调用方信息
        let caller = request
            .metadata()
            .get("caller")
            .map(|v| v.to_str().unwrap_or("unknown"))
            .unwrap_or("unknown");
        
        // 记录请求信息
        info!(path = %path, trace_id = %trace_id, caller = %caller, "收到gRPC请求");
        
        Ok(request)
    }
}

/// 创建一个服务层拦截器，用于记录每个gRPC请求
pub struct LoggingLayer;

impl<S> tower::layer::Layer<S> for LoggingLayer
where
    S: tower::Service<tonic::Request<()>> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<tonic::Status>,
    S::Response: Send + 'static,
{
    type Service = LoggingService<S>;

    fn layer(&self, service: S) -> Self::Service {
        LoggingService {
            inner: service,
        }
    }
}

/// 包装原始服务的日志服务
pub struct LoggingService<S> {
    inner: S,
}

impl<S> tower::Service<tonic::Request<()>> for LoggingService<S>
where
    S: tower::Service<tonic::Request<()>> + Send,
    S::Future: Send + 'static,
    S::Error: Into<tonic::Status>,
    S::Response: Send + 'static,
{
    type Response = S::Response;
    type Error = tonic::Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: tonic::Request<()>) -> Self::Future {
        // 获取请求的路径，通过metadata中的:path字段
        let path = req
            .metadata()
            .get(":path")
            .map(|v| v.to_str().unwrap_or("/unknown"))
            .unwrap_or("/unknown")
            .to_string();
        
        let future = self.inner.call(req);
        
        // 包装原始future，增加日志功能
        Box::pin(async move {
            // 等待原始future完成
            match future.await.map_err(Into::into) {
                Ok(response) => {
                    // 记录成功响应
                    info!(path = %path, "gRPC请求处理成功");
                    Ok(response)
                }
                Err(status) => {
                    // 记录错误响应
                    info!(
                        path = %path, 
                        code = %status.code() as i32, 
                        message = %status.message(), 
                        "gRPC请求处理失败"
                    );
                    Err(status)
                }
            }
        })
    }
} 