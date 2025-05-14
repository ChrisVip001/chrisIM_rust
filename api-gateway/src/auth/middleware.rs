use axum::{
    middleware::Next,
    response::Response,
    http::Request,
    body::{Bytes, Body},
};
use common::error::Error;
use http_body_util::BodyExt;

/// 认证中间件处理函数
pub async fn auth_middleware<B>(request: Request<B>, next: Next) -> Result<Response, Error> 
where 
    B: axum::body::HttpBody<Data = Bytes> + Send + 'static,
    B::Error: std::fmt::Display + Send + Sync + 'static
{
    // 收集请求体并创建新的请求实例
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await
        .map_err(|e| Error::Internal(format!("无法读取请求体: {}", e)))?
        .to_bytes();
    
    let new_body = Body::from(bytes);
    let new_request = Request::from_parts(parts, new_body);
    
    // 调用统一认证入口
    crate::auth::authenticate(new_request, next).await
}
