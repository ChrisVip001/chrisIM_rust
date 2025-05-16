use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
    response::IntoResponse,
    Json,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
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

/// 通用响应生成辅助函数 - 成功响应
fn success_response<T: serde::Serialize>(data: T, status_code: StatusCode) -> Response<Body> {
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "data": data,
            "success": true
        })),
    ).into_response()
}

/// 通用响应生成辅助函数 - 成功带消息
fn success_with_message<T: serde::Serialize>(data: T, message: &str, status_code: StatusCode) -> Response<Body> {
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "data": data,
            "message": message,
            "success": true
        })),
    ).into_response()
}

/// 通用响应生成辅助函数 - 错误响应
fn error_response(message: &str, status_code: StatusCode) -> Response<Body> {
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "message": message,
            "success": false
        })),
    ).into_response()
}

/// 参数提取辅助函数 - 从JSON中提取字符串参数
fn extract_string_param(body: &Value, param_name: &str, alt_name: Option<&str>) -> Result<String, anyhow::Error> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("参数 {} 缺失或格式错误", param_name))
}

/// 参数提取辅助函数 - 从JSON中提取可选字符串参数
fn get_optional_string(body: &Value, param_name: &str, alt_name: Option<&str>) -> Option<String> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        })
}

/// 参数提取辅助函数 - 从JSON中提取i64整数参数
fn get_i64_param(body: &Value, param_name: &str, default: i64) -> i64 {
    body.get(param_name)
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
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

        let service_name = parts.get(2).map_or("unknown".to_string(), |s| s.to_string());
        let method_name = parts.get(3).map_or("unknown".to_string(), |s| s.to_string());

        // 转换服务名为 gRPC 服务名
        let grpc_service = match service_name.as_str() {
            "users" => "user".to_string(),
            "friends" => "friend".to_string(),
            "groups" => "group".to_string(),
            _ => service_name.clone(),
        };

        (service_name, grpc_service, method_name)
    }

    /// 将请求体和URL参数合并到一个Value中
    async fn extract_request_body(req: Request<Body>) -> Result<(Method, String, Value), anyhow::Error> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| q.to_string());

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

        Ok((method, path, body))
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
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.user_client.get_user(&user_id).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_response(convert_user_to_json(&user), StatusCode::OK))
            }

            // 用户名查询
            (&Method::GET, "getUserByUsername") => {
                let username = extract_string_param(&body, "username", None)?;

                let response = self.user_client.get_user_by_username(&username).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_response(convert_user_to_json(&user), StatusCode::OK))
            }

            // 创建用户
            (&Method::POST, "createUser") | (&Method::POST, "register") => {
                let username = body.get("username").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("用户名不能为空"))?;
                let password = body.get("password").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("密码不能为空"))?;

                if username.is_empty() || password.is_empty() {
                    return Err(anyhow::anyhow!("用户名和密码不能为空"));
                }

                let email = body.get("email").and_then(|v| v.as_str()).unwrap_or_default();
                let nickname = body.get("nickname").and_then(|v| v.as_str()).unwrap_or_default();
                let avatar_url = body.get("avatarUrl").or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str()).unwrap_or_default();

                let request = proto::user::CreateUserRequest {
                    username: username.to_string(),
                    email: email.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    avatar_url: avatar_url.to_string(),
                };

                let response = self.user_client.create_user(request).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_with_message(
                    convert_user_to_json(&user),
                    "用户创建成功",
                    StatusCode::CREATED
                ))
            }

            // 更新用户
            (&Method::PUT, "updateUser") | (&Method::PATCH, "updateUser") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let nickname = get_optional_string(&body, "nickname", None);
                let email = get_optional_string(&body, "email", None);
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url"));
                let password = get_optional_string(&body, "password", None);

                let request = proto::user::UpdateUserRequest {
                    user_id,
                    nickname,
                    email,
                    avatar_url,
                    password,
                };

                let response = self.user_client.update_user(request).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_with_message(
                    convert_user_to_json(&user),
                    "用户更新成功",
                    StatusCode::OK
                ))
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
                Err(anyhow::anyhow!("未实现的方法: {}", method_name))
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
                let message = extract_string_param(&body, "message", Some("message"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;
                
                let response = self.friend_client.send_friend_request(&user_id, &friend_id,&message).await?;
                let friendship = response.friendship.ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;

                Ok(success_response(convert_friendship_to_json(&friendship), StatusCode::OK))
            }

            // 接受好友请求
            (&Method::POST, "acceptRequest") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;

                let response = self.friend_client.accept_friend_request(&user_id, &friend_id).await?;
                let friendship = response.friendship.ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;

                Ok(success_response(convert_friendship_to_json(&friendship), StatusCode::OK))
            }

            // 拒绝好友请求
            (&Method::POST, "rejectRequest") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;

                let response = self.friend_client.reject_friend_request(&user_id, &friend_id).await?;
                let friendship = response.friendship.ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;

                Ok(success_response(convert_friendship_to_json(&friendship), StatusCode::OK))
            }

            // 获取好友列表
            (&Method::GET, "getList") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.friend_client.get_friend_list(&user_id).await?;
                let friends = response.friends.iter().map(convert_friend_to_json).collect::<Vec<_>>();

                Ok(success_response(friends, StatusCode::OK))
            }

            // 获取好友请求列表
            (&Method::GET, "getRequests") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.friend_client.get_friend_requests(&user_id).await?;
                let requests = response.requests.iter().map(convert_friendship_to_json).collect::<Vec<_>>();

                Ok(success_response(requests, StatusCode::OK))
            }

            // 删除好友
            (&Method::DELETE, "delete") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;

                let response = self.friend_client.delete_friend(&user_id, &friend_id).await?;

                Ok(success_response(json!({"success": response.success}), StatusCode::OK))
            }

            // 检查好友关系
            (&Method::GET, "checkFriendship") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;

                let response = self.friend_client.check_friendship(&user_id, &friend_id).await?;

                let status_text = match response.status {
                    0 => "PENDING",
                    1 => "ACCEPTED",
                    2 => "REJECTED",
                    3 => "BLOCKED",
                    _ => "UNKNOWN"
                };

                Ok(success_response(
                    json!({
                        "status": response.status,
                        "statusText": status_text
                    }),
                    StatusCode::OK
                ))
            }

            // 其他未实现的方法
            _ => {
                error!("好友服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("好友服务不支持的方法: {}", method_name))
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
                let name = extract_string_param(&body, "name", None)?;
                let owner_id = extract_string_param(&body, "ownerId", Some("owner_id"))?;
                
                let description = body.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                
                let avatar_url = body.get("avatarUrl")
                    .or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let response = self.group_client.create_group(
                    &name,
                    description,
                    &owner_id,
                    avatar_url
                ).await?;

                let group = response.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(convert_group_to_json(&group), StatusCode::OK))
            }

            // 获取群组信息
            (&Method::GET, "getInfo") | (&Method::GET, "get") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.group_client.get_group(&group_id).await?;
                let group = response.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(convert_group_to_json(&group), StatusCode::OK))
            }

            // 更新群组信息
            (&Method::PUT, "update") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                
                let name = get_optional_string(&body, "name", None);
                let description = get_optional_string(&body, "description", None);
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url"));

                let response = self.group_client.update_group(
                    &group_id,
                    name,
                    description,
                    avatar_url
                ).await?;
                
                let group = response.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(convert_group_to_json(&group), StatusCode::OK))
            }

            // 删除群组
            (&Method::DELETE, "delete") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.group_client.delete_group(&group_id, &user_id).await?;

                Ok(success_response(
                    json!({"success": response.success}),
                    StatusCode::OK
                ))
            }

            // 添加成员
            (&Method::POST, "addMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let added_by_id = extract_string_param(&body, "addedById", Some("added_by_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                let response = self.group_client.add_member(&group_id, &user_id, &added_by_id, role).await?;
                let member = response.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(convert_member_to_json(&member), StatusCode::OK))
            }

            // 移除成员
            (&Method::DELETE, "removeMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let removed_by_id = extract_string_param(&body, "removedById", Some("removed_by_id"))?;

                let response = self.group_client.remove_member(&group_id, &user_id, &removed_by_id).await?;
                
                Ok(success_response(
                    json!({"success": response.success}),
                    StatusCode::OK
                ))
            }

            // 更新成员角色
            (&Method::PUT, "updateMemberRole") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let updated_by_id = extract_string_param(&body, "updatedById", Some("updated_by_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                let response = self.group_client.update_member_role(&group_id, &user_id, &updated_by_id, role).await?;
                let member = response.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(convert_member_to_json(&member), StatusCode::OK))
            }

            // 获取群组成员列表
            (&Method::GET, "getMembers") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.group_client.get_members(&group_id).await?;
                let members = response.members.iter().map(convert_member_to_json).collect::<Vec<_>>();

                Ok(success_response(members, StatusCode::OK))
            }

            // 获取用户加入的群组列表
            (&Method::GET, "getUserGroups") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.group_client.get_user_groups(&user_id).await?;
                let groups = response.groups.iter().map(convert_user_group_to_json).collect::<Vec<_>>();

                Ok(success_response(groups, StatusCode::OK))
            }

            // 检查用户是否在群组中
            (&Method::GET, "checkMembership") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.group_client.check_membership(&group_id, &user_id).await?;

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

                Ok(success_response(
                    json!({
                        "isMember": response.is_member,
                        "role": response.role,
                        "roleText": role_text
                    }),
                    StatusCode::OK
                ))
            }

            // 其他未实现的方法
            _ => {
                error!("群组服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("群组服务不支持的方法: {}", method_name))
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

        Box::pin(async move {
            debug!("收到gRPC转发请求，目标: {}", target_url);

            // 提取请求信息
            let (method, path, body) = match Self::extract_request_body(req).await {
                Ok(data) => data,
                Err(err) => {
                    error!("请求解析失败: {}", err);
                    return error_response(&format!("请求解析失败: {}", err), StatusCode::BAD_REQUEST);
                }
            };

            // 解析服务类型
            let (service_name, _, _) = self_clone.parse_path(&path);

            // 根据服务类型调用对应的处理方法
            match service_name.as_str() {
                "users" => self_clone.handle_user_request(&method, &path, body).await
                    .unwrap_or_else(|err| {
                        error!("处理用户服务请求失败: {}", err);
                        error_response(&format!("处理用户服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    }),
                "friends" => self_clone.handle_friend_request(&method, &path, body).await
                    .unwrap_or_else(|err| {
                        error!("处理好友服务请求失败: {}", err);
                        error_response(&format!("处理好友服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    }),
                "groups" => self_clone.handle_group_request(&method, &path, body).await
                    .unwrap_or_else(|err| {
                        error!("处理群组服务请求失败: {}", err);
                        error_response(&format!("处理群组服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    }),
                // 将来可以添加其他服务的处理分支
                _ => {
                    error!("不支持的服务类型: {}", service_name);
                    error_response(
                        &format!("服务 {} 的gRPC转发尚未实现", service_name),
                        StatusCode::NOT_IMPLEMENTED
                    )
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

/// 克隆实现
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
