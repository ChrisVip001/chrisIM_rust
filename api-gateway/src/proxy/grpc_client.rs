use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
    response::IntoResponse,
    Json,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::collections::HashMap;
use tonic::transport::Channel;
use tracing::{debug, error};

use common::proto;
use common::service_registry::ServiceRegistry;

/// gRPC客户端工厂接口
pub trait GrpcClientFactory: Send + Sync {
    /// 转发gRPC请求
    fn forward_request(
        &self,
        req: Request<Body>,
        target_url: String,
    ) -> BoxFuture<'static, Response<Body>>;

    /// 检查健康状态
    fn check_health(&self) -> BoxFuture<'static, bool>;
}

/// gRPC客户端配置
#[derive(Debug, Clone)]
pub struct GrpcClientConfig {
    /// 连接超时（秒）
    pub connect_timeout_secs: u64,
    /// 请求超时（秒）
    pub timeout_secs: u64,
    /// 并发限制
    pub concurrency_limit: usize,
    /// 是否启用负载均衡
    pub enable_load_balancing: bool,
}

impl Default for GrpcClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 5,
            timeout_secs: 30,
            concurrency_limit: 100,
            enable_load_balancing: true,
        }
    }
}

/// 基础gRPC客户端
pub struct BaseGrpcClient {
    channel: Channel,
}

impl BaseGrpcClient {
    /// 创建新的gRPC客户端
    pub async fn new(
        target_url: &str,
        config: GrpcClientConfig,
    ) -> Result<Self, tonic::transport::Error> {
        let endpoint = tonic::transport::Endpoint::new(target_url.to_string())?
            .connect_timeout(std::time::Duration::from_secs(config.connect_timeout_secs))
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .concurrency_limit(config.concurrency_limit);

        let channel = endpoint.connect().await?;

        Ok(Self { channel })
    }

    /// 获取共享通道
    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }
}

/// 通用gRPC客户端工厂
pub struct GrpcClientFactoryImpl {
    // 服务注册表
    service_registry: ServiceRegistry,
    // 各服务客户端
    user_client: common::grpc_client::user_client::UserServiceGrpcClient,
    // 其他服务客户端可以在此添加
}

impl GrpcClientFactoryImpl {
    /// 创建新的通用gRPC客户端工厂
    pub fn new() -> Self {
        let service_registry = ServiceRegistry::from_env();
        let user_client = common::grpc_client::user_client::UserServiceGrpcClient::from_env();

        Self {
            service_registry,
            user_client,
        }
    }

    /// 解析请求路径获取服务和方法名
    fn parse_path(&self, path: &str) -> (String, String, String) {
        // 解析路径格式: /api/[service]/[method]
        let parts: Vec<&str> = path.split('/').collect();

        let service_name = if parts.len() > 2 {
            parts[2].to_string()
        } else {
            "unknown".to_string()
        };
        let method_name = if parts.len() > 3 {
            parts[3].to_string()
        } else {
            "unknown".to_string()
        };

        // 转换服务名为 gRPC 服务名
        let grpc_service = match service_name.as_str() {
            "users" => "user".to_string(),
            "friends" => "friend".to_string(),
            "groups" => "group".to_string(),
            "auth" => "auth".to_string(),
            _ => service_name.clone(),
        };

        (service_name, grpc_service, method_name)
    }

    /// 将HTTP请求转换为用户服务gRPC请求
    async fn handle_user_request(
        &self,
        method: Method,
        path: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理用户服务请求: {} {}", method, path);

        let (_, _, method_name) = self.parse_path(path);

        match (method, method_name.as_str()) {
            // 用户查询
            (Method::GET, "getUserById") | (Method::GET, "getUser") => {
                let user_id = match body.get("userId").or_else(|| body.get("user_id")) {
                    Some(id) => id.as_str().unwrap_or_default().to_string(),
                    None => {
                        return Ok((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "code": 400,
                                "message": "缺少用户ID参数",
                                "success": false
                            })),
                        )
                            .into_response());
                    }
                };

                match self.user_client.get_user(&user_id).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_user_to_json(&user),
                                "success": true
                            })),
                        )
                            .into_response())
                    }
                    Err(err) => {
                        error!("获取用户失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取用户失败: {}", err),
                                "success": false
                            })),
                        )
                            .into_response())
                    }
                }
            }

            // 用户名查询
            (Method::GET, "getUserByUsername") => {
                let username = match body.get("username") {
                    Some(name) => name.as_str().unwrap_or_default().to_string(),
                    None => {
                        return Ok((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "code": 400,
                                "message": "缺少username参数",
                                "success": false
                            })),
                        )
                            .into_response());
                    }
                };

                match self.user_client.get_user_by_username(&username).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_user_to_json(&user),
                                "success": true
                            })),
                        )
                            .into_response())
                    }
                    Err(err) => {
                        error!("获取用户失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取用户失败: {}", err),
                                "success": false
                            })),
                        )
                            .into_response())
                    }
                }
            }

            // 创建用户
            (Method::POST, "createUser") | (Method::POST, "register") => {
                let username = body
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let email = body
                    .get("email")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let password = body
                    .get("password")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let nickname = body
                    .get("nickname")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let avatar_url = body
                    .get("avatarUrl")
                    .or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if username.is_empty() || password.is_empty() {
                    return Ok((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": 400,
                            "message": "用户名和密码不能为空",
                            "success": false
                        })),
                    )
                        .into_response());
                }

                let request = proto::user::CreateUserRequest {
                    username: username.to_string(),
                    email: email.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    avatar_url: avatar_url.to_string(),
                };

                match self.user_client.create_user(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok((
                            StatusCode::CREATED,
                            Json(json!({
                                "code": 201,
                                "data": convert_user_to_json(&user),
                                "success": true,
                                "message": "用户创建成功"
                            })),
                        )
                            .into_response())
                    }
                    Err(err) => {
                        error!("创建用户失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("创建用户失败: {}", err),
                                "success": false
                            })),
                        )
                            .into_response())
                    }
                }
            }

            // 更新用户
            (Method::PUT, "updateUser") | (Method::PATCH, "updateUser") => {
                let user_id = body
                    .get("userId")
                    .or_else(|| body.get("user_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if user_id.is_empty() {
                    return Ok((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": 400,
                            "message": "用户ID不能为空",
                            "success": false
                        })),
                    )
                        .into_response());
                }

                let nickname = body
                    .get("nickname")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let email = body
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let avatar_url = body
                    .get("avatarUrl")
                    .or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let password = body
                    .get("password")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let request = proto::user::UpdateUserRequest {
                    user_id: user_id.to_string(),
                    nickname,
                    email,
                    avatar_url,
                    password,
                };

                match self.user_client.update_user(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_user_to_json(&user),
                                "success": true,
                                "message": "用户更新成功"
                            })),
                        )
                            .into_response())
                    }
                    Err(err) => {
                        error!("更新用户失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("更新用户失败: {}", err),
                                "success": false
                            })),
                        )
                            .into_response())
                    }
                }
            }

            // 其他未知方法
            _ => {
                error!("未知的用户服务方法: {}", method_name);
                Ok((
                    StatusCode::NOT_IMPLEMENTED,
                    Json(json!({
                        "code": 501,
                        "message": format!("未实现的方法: {}", method_name),
                        "success": false
                    })),
                )
                    .into_response())
            }
        }
    }

    // 未来可以添加其他服务的处理方法，如：
    // async fn handle_friend_request(...)
    // async fn handle_group_request(...)
    // async fn handle_auth_request(...)
}

impl GrpcClientFactory for GrpcClientFactoryImpl {
    fn forward_request(
        &self,
        req: Request<Body>,
        target_url: String,
    ) -> BoxFuture<'static, Response<Body>> {
        let self_clone = self.clone();

        Box::pin(async move {
            debug!("收到gRPC转发请求，目标: {}", target_url);

            // 获取请求参数
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let query = req.uri().query().map(|q| q.to_string());

            // 提取请求体
            let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    error!("读取请求体失败: {}", err);
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": 400,
                            "message": format!("读取请求体失败: {}", err),
                            "success": false
                        })),
                    )
                        .into_response();
                }
            };

            // 解析JSON请求体
            let body: Value = match serde_json::from_slice(&body_bytes) {
                Ok(json) => json,
                Err(_) => {
                    // 尝试从URL参数获取
                    match query {
                        Some(query_str) => {
                            let mut map = HashMap::new();
                            for param in query_str.split('&') {
                                if let Some((key, value)) = param.split_once('=') {
                                    map.insert(key.to_string(), Value::String(value.to_string()));
                                }
                            }
                            Value::Object(serde_json::map::Map::from_iter(map.into_iter()))
                        }
                        None => Value::Object(serde_json::map::Map::new()),
                    }
                }
            };

            // 解析服务类型
            let (service_name, _, _) = self_clone.parse_path(&path);

            // 根据服务类型调用对应的处理方法
            match service_name.as_str() {
                "users" => match self_clone.handle_user_request(method, &path, body).await {
                    Ok(response) => response,
                    Err(err) => {
                        error!("处理用户服务请求失败: {}", err);
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("处理请求失败: {}", err),
                                "success": false
                            })),
                        )
                            .into_response()
                    }
                },
                // 将来可以添加其他服务的处理分支
                // "friends" => self_clone.handle_friend_request(method, &path, body).await,
                // "groups" => self_clone.handle_group_request(method, &path, body).await,
                // "auth" => self_clone.handle_auth_request(method, &path, body).await,
                _ => {
                    error!("不支持的服务类型: {}", service_name);
                    (
                        StatusCode::NOT_IMPLEMENTED,
                        Json(json!({
                            "code": 501,
                            "message": format!("服务 {} 的gRPC转发尚未实现", service_name),
                            "target": target_url,
                            "success": false
                        })),
                    )
                        .into_response()
                }
            }
        })
    }

    fn check_health(&self) -> BoxFuture<'static, bool> {
        // 克隆必要的数据以避免生命周期问题
        let service_registry = self.service_registry.clone();

        Box::pin(async move {
            // 简单的健康检查：尝试连接用户服务
            match service_registry.discover_service("user-service").await {
                Ok(_) => true,
                Err(_) => false,
            }
        })
    }
}

// 克隆实现
impl Clone for GrpcClientFactoryImpl {
    fn clone(&self) -> Self {
        Self {
            service_registry: self.service_registry.clone(),
            user_client: self.user_client.clone(),
        }
    }
}

/// 将用户消息转换为JSON
fn convert_user_to_json(user: &proto::user::User) -> Value {
    // 转换时间戳
    let created_at = user
        .created_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let updated_at = user
        .updated_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    json!({
        "id": user.id,
        "username": user.username,
        "email": user.email,
        "nickname": user.nickname,
        "avatarUrl": user.avatar_url,
        "createdAt": created_at,
        "updatedAt": updated_at
    })
}

/// 创建gRPC通道
pub async fn create_grpc_channel(target_url: &str) -> Result<Channel, tonic::transport::Error> {
    let endpoint = tonic::transport::Endpoint::new(target_url.to_string())?
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .concurrency_limit(100);

    endpoint.connect().await
}
