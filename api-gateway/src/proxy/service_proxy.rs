use crate::auth::jwt::UserInfo;
use crate::proxy::services::{
    CommonServiceHandler, FriendServiceHandler, GroupServiceHandler, UserServiceHandler,
};
use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
    response::IntoResponse,
};
use common::configs::routes_config::ServiceType;
use serde_json::{json, Value};

/// 服务代理 - 负责转发请求到后端服务
#[derive(Clone)]
pub struct ServiceProxy;

impl ServiceProxy {
    /// 转发请求到后端服务
    pub async fn forward_request(req: Request<Body>, service_type: &ServiceType) -> Response<Body> {
        match service_type {
            // 核心业务服务使用gRPC转发（通过Handler处理HTTP到gRPC的转换）
            ServiceType::User => forward_user_request(req).await,
            ServiceType::Friend => forward_friend_request(req).await,
            ServiceType::Group => forward_group_request(req).await,
            ServiceType::Chat => forward_chat_request(req).await,
            ServiceType::Common => forward_common_request(req).await,

            // HTTP服务或静态服务使用HTTP转发
            ServiceType::Static => forward_http_request(req, "static").await,
        }
    }
}
/// 转发用户服务请求
async fn forward_user_request(req: Request<Body>) -> Response<Body> {
    // 解析请求
    let (method, path, body, user_info) = match extract_request_data(req).await {
        Ok(data) => data,
        Err(err) => return error_response(&err.to_string(), StatusCode::BAD_REQUEST),
    };

    // 使用Handler处理HTTP到gRPC的转换
    UserServiceHandler::handle_request(&method, &path, body, user_info)
        .await
        .unwrap_or_else(|err| error_response(&err.to_string(), StatusCode::INTERNAL_SERVER_ERROR))
}

/// 转发好友服务请求
async fn forward_friend_request(req: Request<Body>) -> Response<Body> {
    let (method, path, body, user_info) = match extract_request_data(req).await {
        Ok(data) => data,
        Err(err) => return error_response(&err.to_string(), StatusCode::BAD_REQUEST),
    };

    FriendServiceHandler::handle_request(&method, &path, body, user_info)
        .await
        .unwrap_or_else(|err| error_response(&err.to_string(), StatusCode::INTERNAL_SERVER_ERROR))
}

/// 转发群组服务请求
async fn forward_group_request(req: Request<Body>) -> Response<Body> {
    let (method, path, body, user_info) = match extract_request_data(req).await {
        Ok(data) => data,
        Err(err) => return error_response(&err.to_string(), StatusCode::BAD_REQUEST),
    };

    GroupServiceHandler::handle_request(&method, &path, body, user_info)
        .await
        .unwrap_or_else(|err| error_response(&err.to_string(), StatusCode::INTERNAL_SERVER_ERROR))
}

/// 转发聊天服务请求
async fn forward_chat_request(_req: Request<Body>) -> Response<Body> {
    error_response("聊天服务暂未实现", StatusCode::NOT_IMPLEMENTED)
}

/// 转发通用服务请求（组合多个服务）
async fn forward_common_request(req: Request<Body>) -> Response<Body> {
    let (method, path, body, user_info) = match extract_request_data(req).await {
        Ok(data) => data,
        Err(err) => return error_response(&err.to_string(), StatusCode::BAD_REQUEST),
    };

    CommonServiceHandler::handle_request(&method, &path, body, user_info)
        .await
        .unwrap_or_else(|err| error_response(&err.to_string(), StatusCode::INTERNAL_SERVER_ERROR))
}

/// 提取请求数据
async fn extract_request_data(
    req: Request<Body>,
) -> Result<(Method, String, Value, Option<UserInfo>), anyhow::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());
    let user_info = req.extensions().get::<UserInfo>().cloned();

    // 提取请求体
    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|e| anyhow::anyhow!("读取请求体失败: {}", e))?;

    // 解析JSON请求体或URL参数
    let body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(json) => json,
        Err(_) => {
            // 尝试从URL参数获取
            let mut map = serde_json::map::Map::new();
            if let Some(query_str) = query {
                for param in query_str.split('&') {
                    if let Some((key, value)) = param.split_once('=') {
                        map.insert(key.to_string(), Value::String(value.to_string()));
                    }
                }
            }
            Value::Object(map)
        }
    };

    Ok((method, path, body, user_info))
}

/// 转发HTTP请求
async fn forward_http_request(_req: Request<Body>, service_name: &str) -> Response<Body> {
    // HTTP转发逻辑保持不变
    error_response(
        &format!("{} HTTP转发待实现", service_name),
        StatusCode::NOT_IMPLEMENTED,
    )
}

/// 通用错误响应
fn error_response(message: &str, status_code: StatusCode) -> Response<Body> {
    (
        status_code,
        axum::Json(json!({
            "code": status_code.as_u16(),
            "message": message,
            "success": false
        })),
    )
        .into_response()
}

/// 通用成功响应
fn success_response<T: serde::Serialize>(data: T, status_code: StatusCode) -> Response<Body> {
    (
        status_code,
        axum::Json(json!({
            "code": status_code.as_u16(),
            "data": data,
            "success": true
        })),
    )
        .into_response()
}
