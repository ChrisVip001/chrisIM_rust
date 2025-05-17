use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use tonic::transport::Channel;
use tracing::{debug, error};
use common::grpc_client::{FriendServiceGrpcClient, GroupServiceGrpcClient, UserServiceGrpcClient};
use common::service_registry::ServiceRegistry;

use crate::proxy::services::{
    UserServiceHandler, FriendServiceHandler, GroupServiceHandler,
    common::error_response
};

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
    // 各服务处理器
    user_service: UserServiceHandler,
    friend_service: FriendServiceHandler,
    group_service: GroupServiceHandler,
}

impl GrpcClientFactoryImpl {
    /// 创建新的通用gRPC客户端工厂
    pub fn new() -> Self {
        let service_registry = ServiceRegistry::from_env();

        // 创建各服务客户端
        let user_client = UserServiceGrpcClient::from_env();
        let friend_client = FriendServiceGrpcClient::from_env();
        let group_client = GroupServiceGrpcClient::from_env();

        // 创建各服务处理器
        let user_service = UserServiceHandler::new(user_client);
        let friend_service = FriendServiceHandler::new(friend_client);
        let group_service = GroupServiceHandler::new(group_client);

        Self {
            service_registry,
            user_service,
            friend_service,
            group_service,
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
                "users" => self_clone.user_service.handle_request(&method, &path, body).await
                    .unwrap_or_else(|err| {
                        error!("处理用户服务请求失败: {}", err);
                        error_response(&format!("处理用户服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    }),
                "friends" => self_clone.friend_service.handle_request(&method, &path, body).await
                    .unwrap_or_else(|err| {
                        error!("处理好友服务请求失败: {}", err);
                        error_response(&format!("处理好友服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    }),
                "groups" => self_clone.group_service.handle_request(&method, &path, body).await
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
            user_service: self.user_service.clone(),
            friend_service: self.friend_service.clone(),
            group_service: self.group_service.clone(),
        }
    }
}

/// 创建gRPC通道
pub async fn create_grpc_channel(target_url: &str) -> Result<Channel, tonic::transport::Error> {
    let endpoint = tonic::transport::Endpoint::new(target_url.to_string())?
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .concurrency_limit(100);

    endpoint.connect().await
}
