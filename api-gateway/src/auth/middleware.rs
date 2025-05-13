use axum::{
    middleware::Next,
    response::Response,
    http::Request,
    body::{Bytes, Body},
};
use common::error::Error;
use crate::auth::jwt::UserInfo;
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

/// 权限验证中间件
pub async fn authorize<B>(
    request: Request<B>,
    next: Next,
    required_roles: Vec<String>
) -> Result<Response, Error> 
where 
    B: axum::body::HttpBody<Data = Bytes> + Send + 'static,
    B::Error: std::fmt::Display + Send + Sync + 'static
{
    // 从请求扩展中获取用户信息
    let user = request.extensions()
        .get::<UserInfo>()
        .cloned()
        .ok_or(Error::Unauthorized)?;
    
    // 检查用户角色
    if !required_roles.is_empty() && !has_required_roles(&user.roles, &required_roles) {
        return Err(Error::InsufficientPermissions);
    }
    
    // 转换请求体类型
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await
        .map_err(|_| Error::Internal("无法读取请求体".to_string()))?
        .to_bytes();
    let new_body = Body::from(bytes);
    let new_request = Request::from_parts(parts, new_body);
    
    // 继续处理请求
    Ok(next.run(new_request).await)
}

/// 检查用户是否具有所需角色
fn has_required_roles(user_roles: &[String], required_roles: &[String]) -> bool {
    // 如果用户具有admin角色，直接返回true
    if user_roles.iter().any(|r| r == "admin" || r == "ADMIN") {
        return true;
    }
    
    // 检查用户是否拥有所需的任意一个角色
    required_roles.iter().any(|required| user_roles.contains(required))
}

/// 创建需要特定角色的权限中间件
// pub fn require_roles(
//     roles: Vec<String>
// ) -> axum::middleware::from_fn_with_state<(), impl Fn(Request<Body>, Next) -> Result<Response, Error>> {
//     axum::middleware::from_fn(move |request: Request<Body>, next: Next| {
//         let roles = roles.clone();
//         async move {
//             authorize(request, next, roles).await
//         }
//     })
// }

/// 从请求中获取用户信息
pub fn get_user_from_request<B>(request: &Request<B>) -> Option<UserInfo> {
    request.extensions().get::<UserInfo>().cloned()
} 