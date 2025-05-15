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
use common::grpc_client::{FriendServiceGrpcClient, GroupServiceGrpcClient, UserServiceGrpcClient};
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
    user_client: UserServiceGrpcClient,
    // 其他服务客户端可以在此添加
    friend_client: FriendServiceGrpcClient,
    group_client: GroupServiceGrpcClient,
}

impl GrpcClientFactoryImpl {
    /// 创建新的通用gRPC客户端工厂
    pub fn new() -> Self {
        let service_registry = ServiceRegistry::from_env();
        let user_client = UserServiceGrpcClient::from_env();
        let friend_client = FriendServiceGrpcClient::from_env();
        let group_client = GroupServiceGrpcClient::from_env();

        Self {
            service_registry,
            user_client,
            friend_client,
            group_client,
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
        method: &Method,
        path: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理用户服务请求: {} {}", method, path);

        let (_, _, method_name) = self.parse_path(path);

        match (method, method_name.as_str()) {
            // 用户查询
            (&Method::GET, "getUserById") | (&Method::GET, "getUser") => {
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
            (&Method::GET, "getUserByUsername") => {
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
            (&Method::POST, "createUser") | (&Method::POST, "register") => {
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
            (&Method::PUT, "updateUser") | (&Method::PATCH, "updateUser") => {
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

            // 用户账号密码注册
            (&Method::POST, "registerByUsername") => {
                let username = body
                    .get("username")
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
                let tenant_id = body
                    .get("tenant_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let phone = body
                    .get("phone")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if username.is_empty() || password.is_empty() {
                    return Ok((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": 400,
                            "message": "用户名或者密码不能为空",
                            "success": false
                        })),
                    )
                    .into_response());
                }

                let request = proto::user::RegisterRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string()
                };

                match self.user_client.register_by_username(request).await {
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
                                "message": "用户注册成功"
                            })),
                        )
                        .into_response())
                    }
                    Err(err) => {
                        error!("注册用户失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("注册用户失败: {}", err),
                                "success": false
                            })),
                        )
                        .into_response())
                    }
                }
            }

            // 用户手机号注册
            (&Method::POST, "registerByPhone") => {
                let username = body
                    .get("username")
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
                let tenant_id = body
                    .get("tenant_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let phone = body
                    .get("phone")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if phone.is_empty() || password.is_empty() {
                    return Ok((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": 400,
                            "message": "手机号或者密码不能为空",
                            "success": false
                        })),
                    )
                        .into_response());
                }

                let request = proto::user::RegisterRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string()
                };

                match self.user_client.register_by_phone(request).await {
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
                                "message": "用户注册成功"
                            })),
                        )
                        .into_response())
                    }
                    Err(err) => {
                        error!("注册用户失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("注册用户失败: {}", err),
                                "success": false
                            })),
                        )
                        .into_response())
                    }
                }
            }

            // 忘记密码
            (&Method::POST, "forgetPassword") => {
                let username = body
                    .get("username")
                    .or_else(|| body.get("username"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let phone = body
                    .get("phone")
                    .or_else(|| body.get("phone"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if username.is_empty() && phone.is_empty() {
                    return Ok((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": 400,
                            "message": "用户名或者手机号不能为空",
                            "success": false
                        })),
                    )
                    .into_response());
                }

                let password = body
                    .get("password")
                    .or_else(|| body.get("password"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let tenant_id = body
                    .get("tenant_id")
                    .or_else(|| body.get("tenant_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let request = proto::user::ForgetPasswordRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string(),
                };

                match self.user_client.forget_password(request).await {
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
                                "message": "密码更新成功"
                            })),
                        )
                            .into_response())
                    }
                    Err(err) => {
                        error!("密码更新失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("密码更新失败: {}", err),
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

    /// 处理好友服务请求
    async fn handle_friend_request(
        &self,
        method: &Method,
        path: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理好友服务请求: {} {}", method, path);

        let (_, _, method_name) = self.parse_path(path);

        match (method, method_name.as_str()) {
            // 发送好友请求
            (&Method::POST, "sendRequest") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let friend_id = body.get("friendId").or_else(|| body.get("friend_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少好友ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("好友ID格式错误"))?;

                match self.friend_client.send_friend_request(user_id, friend_id).await {
                    Ok(response) => {
                        let friendship = response.friendship
                            .ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_friendship_to_json(&friendship),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("发送好友请求失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("发送好友请求失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 接受好友请求
            (&Method::POST, "acceptRequest") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let friend_id = body.get("friendId").or_else(|| body.get("friend_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少好友ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("好友ID格式错误"))?;

                match self.friend_client.accept_friend_request(user_id, friend_id).await {
                    Ok(response) => {
                        let friendship = response.friendship
                            .ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_friendship_to_json(&friendship),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("接受好友请求失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("接受好友请求失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 拒绝好友请求
            (&Method::POST, "rejectRequest") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let friend_id = body.get("friendId").or_else(|| body.get("friend_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少好友ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("好友ID格式错误"))?;

                match self.friend_client.reject_friend_request(user_id, friend_id).await {
                    Ok(response) => {
                        let friendship = response.friendship
                            .ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_friendship_to_json(&friendship),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("拒绝好友请求失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("拒绝好友请求失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 获取好友列表
            (&Method::GET, "getList") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;

                match self.friend_client.get_friend_list(user_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": response.friends.iter().map(convert_friend_to_json).collect::<Vec<_>>(),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("获取好友列表失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取好友列表失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 获取好友请求列表
            (&Method::GET, "getRequests") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;

                match self.friend_client.get_friend_requests(user_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": response.requests.iter().map(convert_friendship_to_json).collect::<Vec<_>>(),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("获取好友请求列表失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取好友请求列表失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 删除好友
            (&Method::DELETE, "delete") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let friend_id = body.get("friendId").or_else(|| body.get("friend_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少好友ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("好友ID格式错误"))?;

                match self.friend_client.delete_friend(user_id, friend_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": { "success": response.success },
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("删除好友失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("删除好友失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 检查好友关系
            (&Method::GET, "checkFriendship") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let friend_id = body.get("friendId").or_else(|| body.get("friend_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少好友ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("好友ID格式错误"))?;

                match self.friend_client.check_friendship(user_id, friend_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": {
                                    "status": response.status,
                                    "statusText": match response.status {
                                        0 => "PENDING",
                                        1 => "ACCEPTED",
                                        2 => "REJECTED",
                                        3 => "BLOCKED",
                                        _ => "UNKNOWN"
                                    }
                                },
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("检查好友关系失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("检查好友关系失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 其他未实现的方法
            _ => {
                error!("好友服务不支持的方法: {} {}", method, method_name);
                Ok((
                    StatusCode::NOT_IMPLEMENTED,
                    Json(json!({
                        "code": 501,
                        "message": format!("好友服务不支持的方法: {}", method_name),
                        "success": false
                    })),
                ).into_response())
            }
        }
    }

    /// 处理群组服务请求
    async fn handle_group_request(
        &self,
        method: &Method,
        path: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理群组服务请求: {} {}", method, path);

        let (_, _, method_name) = self.parse_path(path);

        match (method, method_name.as_str()) {
            // 创建群组
            (&Method::POST, "create") => {
                let name = body.get("name")
                    .ok_or_else(|| anyhow::anyhow!("缺少群组名称"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组名称格式错误"))?;
                
                let description = body.get("description")
                    .map(|v| v.as_str().unwrap_or_default())
                    .unwrap_or_default();
                
                let owner_id = body.get("ownerId").or_else(|| body.get("owner_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少拥有者ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("拥有者ID格式错误"))?;
                
                let avatar_url = body.get("avatarUrl").or_else(|| body.get("avatar_url"))
                    .map(|v| v.as_str().unwrap_or_default())
                    .unwrap_or_default();

                match self.group_client.create_group(name, description, owner_id, avatar_url).await {
                    Ok(response) => {
                        let group = response.group
                            .ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_group_to_json(&group),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("创建群组失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("创建群组失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 获取群组信息
            (&Method::GET, "getInfo") | (&Method::GET, "get") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;

                match self.group_client.get_group(group_id).await {
                    Ok(response) => {
                        let group = response.group
                            .ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_group_to_json(&group),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("获取群组信息失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取群组信息失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 更新群组信息
            (&Method::PUT, "update") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;
                
                let name = body.get("name").map(|v| {
                    if v.is_null() { None } else { Some(v.as_str().unwrap_or_default().to_string()) }
                }).unwrap_or(None);
                
                let description = body.get("description").map(|v| {
                    if v.is_null() { None } else { Some(v.as_str().unwrap_or_default().to_string()) }
                }).unwrap_or(None);
                
                let avatar_url = body.get("avatarUrl").or_else(|| body.get("avatar_url")).map(|v| {
                    if v.is_null() { None } else { Some(v.as_str().unwrap_or_default().to_string()) }
                }).unwrap_or(None);

                match self.group_client.update_group(group_id, name, description, avatar_url).await {
                    Ok(response) => {
                        let group = response.group
                            .ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_group_to_json(&group),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("更新群组信息失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("更新群组信息失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 删除群组
            (&Method::DELETE, "delete") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;
                
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;

                match self.group_client.delete_group(group_id, user_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": { "success": response.success },
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("删除群组失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("删除群组失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 添加成员
            (&Method::POST, "addMember") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;
                
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let added_by_id = body.get("addedById").or_else(|| body.get("added_by_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少操作者ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("操作者ID格式错误"))?;
                
                let role_value = body.get("role").and_then(|v| v.as_i64()).unwrap_or(0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                match self.group_client.add_member(group_id, user_id, added_by_id, role).await {
                    Ok(response) => {
                        let member = response.member
                            .ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_member_to_json(&member),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("添加群组成员失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("添加群组成员失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 移除成员
            (&Method::DELETE, "removeMember") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;
                
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let removed_by_id = body.get("removedById").or_else(|| body.get("removed_by_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少操作者ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("操作者ID格式错误"))?;

                match self.group_client.remove_member(group_id, user_id, removed_by_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": { "success": response.success },
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("移除群组成员失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("移除群组成员失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 更新成员角色
            (&Method::PUT, "updateMemberRole") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;
                
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;
                
                let updated_by_id = body.get("updatedById").or_else(|| body.get("updated_by_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少操作者ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("操作者ID格式错误"))?;
                
                let role_value = body.get("role").and_then(|v| v.as_i64()).unwrap_or(0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                match self.group_client.update_member_role(group_id, user_id, updated_by_id, role).await {
                    Ok(response) => {
                        let member = response.member
                            .ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": convert_member_to_json(&member),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("更新成员角色失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("更新成员角色失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 获取群组成员列表
            (&Method::GET, "getMembers") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;

                match self.group_client.get_members(group_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": response.members.iter().map(convert_member_to_json).collect::<Vec<_>>(),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("获取群组成员列表失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取群组成员列表失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 获取用户加入的群组列表
            (&Method::GET, "getUserGroups") => {
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;

                match self.group_client.get_user_groups(user_id).await {
                    Ok(response) => {
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": response.groups.iter().map(convert_user_group_to_json).collect::<Vec<_>>(),
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("获取用户群组列表失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("获取用户群组列表失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 检查用户是否在群组中
            (&Method::GET, "checkMembership") => {
                let group_id = body.get("groupId").or_else(|| body.get("group_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少群组ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("群组ID格式错误"))?;
                
                let user_id = body.get("userId").or_else(|| body.get("user_id"))
                    .ok_or_else(|| anyhow::anyhow!("缺少用户ID"))?
                    .as_str().ok_or_else(|| anyhow::anyhow!("用户ID格式错误"))?;

                match self.group_client.check_membership(group_id, user_id).await {
                    Ok(response) => {
                        let role_text = if response.is_member {
                            match response.role.unwrap_or(0) {
                                0 => "MEMBER",
                                1 => "ADMIN",
                                2 => "OWNER",
                                _ => "UNKNOWN"
                            }
                        } else {
                            "NONE"
                        };
                        
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "code": 200,
                                "data": {
                                    "isMember": response.is_member,
                                    "role": response.role,
                                    "roleText": role_text
                                },
                                "success": true
                            })),
                        ).into_response())
                    }
                    Err(err) => {
                        error!("检查群组成员资格失败: {}", err);
                        Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "code": 500,
                                "message": format!("检查群组成员资格失败: {}", err),
                                "success": false
                            })),
                        ).into_response())
                    }
                }
            }

            // 其他未实现的方法
            _ => {
                error!("群组服务不支持的方法: {} {}", method, method_name);
                Ok((
                    StatusCode::NOT_IMPLEMENTED,
                    Json(json!({
                        "code": 501,
                        "message": format!("群组服务不支持的方法: {}", method_name),
                        "success": false
                    })),
                ).into_response())
            }
        }
    }
}

impl GrpcClientFactory for GrpcClientFactoryImpl {
    fn forward_request(
        &self,
        req: Request<Body>,
        target_url: String,
    ) -> BoxFuture<'static, Response<Body>> {
        let self_clone = self.clone();
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let query = req.uri().query().map(|q| q.to_string());

        Box::pin(async move {
            debug!("收到gRPC转发请求，目标: {}", target_url);

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
                "users" => self_clone.handle_user_request(&method, &path, body).await.unwrap_or_else(|err| {
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
                }),
                "friends" => self_clone.handle_friend_request(&method, &path, body).await.unwrap_or_else(|err| {
                    error!("处理好友服务请求失败: {}", err);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                                "code": 500,
                                "message": format!("处理请求失败: {}", err),
                                "success": false
                            })),
                    )
                        .into_response()
                }),
                "groups" => self_clone.handle_group_request(&method, &path, body).await.unwrap_or_else(|err| {
                    error!("处理群组服务请求失败: {}", err);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                                "code": 500,
                                "message": format!("处理请求失败: {}", err),
                                "success": false
                            })),
                    )
                        .into_response()
                }),
                // 将来可以添加其他服务的处理分支
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
            friend_client: self.friend_client.clone(),
            group_client: self.group_client.clone(),
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

    let last_login_time = user
        .last_login_time
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
        "updatedAt": updated_at,
        "phone" : user.phone,
        "address" : user.address,
        "head_image" : user.head_image,
        "head_image_thumb" : user.head_image_thumb,
        "sex" : user.sex,
        "user_stat" : user.user_stat,
        "tenant_id" : user.tenant_id,
        "last_login_time"  : last_login_time,
        "user_idx" : user.user_idx,
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

/// 将好友关系消息转换为JSON
fn convert_friendship_to_json(friendship: &proto::friend::Friendship) -> Value {
    // 转换时间戳
    let created_at = friendship
        .created_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let updated_at = friendship
        .updated_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let status_text = match friendship.status {
        0 => "PENDING",
        1 => "ACCEPTED",
        2 => "REJECTED",
        3 => "BLOCKED",
        _ => "UNKNOWN"
    };

    json!({
        "id": friendship.id,
        "userId": friendship.user_id,
        "friendId": friendship.friend_id,
        "status": friendship.status,
        "statusText": status_text,
        "createdAt": created_at,
        "updatedAt": updated_at,
    })
}

/// 将好友消息转换为JSON
fn convert_friend_to_json(friend: &proto::friend::Friend) -> Value {
    // 转换时间戳
    let friendship_created_at = friend
        .friendship_created_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    json!({
        "id": friend.id,
        "username": friend.username,
        "nickname": friend.nickname,
        "avatarUrl": friend.avatar_url,
        "friendshipCreatedAt": friendship_created_at,
    })
}

/// 将群组消息转换为JSON
fn convert_group_to_json(group: &proto::group::Group) -> Value {
    // 转换时间戳
    let created_at = group
        .created_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let updated_at = group
        .updated_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    json!({
        "id": group.id,
        "name": group.name,
        "description": group.description,
        "avatarUrl": group.avatar_url,
        "ownerId": group.owner_id,
        "memberCount": group.member_count,
        "createdAt": created_at,
        "updatedAt": updated_at,
    })
}

/// 将群组成员消息转换为JSON
fn convert_member_to_json(member: &proto::group::Member) -> Value {
    // 转换时间戳
    let joined_at = member
        .joined_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let role_text = match member.role {
        0 => "MEMBER",
        1 => "ADMIN",
        2 => "OWNER",
        _ => "UNKNOWN"
    };

    json!({
        "id": member.id,
        "groupId": member.group_id,
        "userId": member.user_id,
        "username": member.username,
        "nickname": member.nickname,
        "avatarUrl": member.avatar_url,
        "role": member.role,
        "roleText": role_text,
        "joinedAt": joined_at,
    })
}

/// 将用户群组消息转换为JSON
fn convert_user_group_to_json(user_group: &proto::group::UserGroup) -> Value {
    // 转换时间戳
    let joined_at = user_group
        .joined_at
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let role_text = match user_group.role {
        0 => "MEMBER",
        1 => "ADMIN",
        2 => "OWNER",
        _ => "UNKNOWN"
    };

    json!({
        "id": user_group.id,
        "name": user_group.name,
        "avatarUrl": user_group.avatar_url,
        "memberCount": user_group.member_count,
        "role": user_group.role,
        "roleText": role_text,
        "joinedAt": joined_at,
    })
}
