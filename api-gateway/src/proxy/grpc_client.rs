use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use futures::future::BoxFuture;
use serde_json::Value;
use tracing::{debug, error};
use common::proto::user::user_service_client::UserServiceClient;
use common::proto::friend::friend_service_client::FriendServiceClient;
use common::proto::group::group_service_client::GroupServiceClient;
use common::grpc_client::{FriendServiceGrpcClient, GroupServiceGrpcClient, UserServiceGrpcClient};
use common::config::{AppConfig, ConfigLoader};
use common::service_discovery::LbWithServiceDiscovery;
use common::grpc_client::base::{service_register_center, get_rpc_client};
use std::sync::{Arc, RwLock};

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

/// 用于服务处理器初始化的函数类型
type ServiceInitializer<T> = Arc<dyn Fn() -> T + Send + Sync>;

/// 延迟初始化的服务处理器包装
struct LazyServiceHandler<T> {
    inner: RwLock<Option<T>>,
    initializer: ServiceInitializer<T>,
}

impl<T: Clone> LazyServiceHandler<T> {
    /// 创建新的延迟初始化包装器
    fn new<F>(initializer: F) -> Self 
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self {
            inner: RwLock::new(None),
            initializer: Arc::new(initializer),
        }
    }

    /// 获取或初始化服务处理器
    fn get(&self) -> T {
        // 先尝试读取
        if let Some(handler) = self.inner.read().unwrap().clone() {
            return handler;
        }

        // 如果不存在，获取写锁并初始化
        let mut write_guard = self.inner.write().unwrap();
        if write_guard.is_none() {
            *write_guard = Some((self.initializer)());
        }
        
        write_guard.clone().unwrap()
    }
}

impl<T: Clone> Clone for LazyServiceHandler<T> {
    fn clone(&self) -> Self {
        Self {
            inner: RwLock::new(self.inner.read().unwrap().clone()),
            initializer: self.initializer.clone(),
        }
    }
}

/// 通用gRPC客户端工厂
pub struct GrpcClientFactoryImpl {
    // 应用配置
    config: Arc<AppConfig>,
    // 服务注册中心
    service_register: Arc<dyn common::service_register_center::ServiceRegister>,
    // 各服务处理器（延迟初始化）
    user_service: LazyServiceHandler<UserServiceHandler>,
    friend_service: LazyServiceHandler<FriendServiceHandler>,
    group_service: LazyServiceHandler<GroupServiceHandler>,
}

impl GrpcClientFactoryImpl {
    /// 创建新的通用gRPC客户端工厂
    pub fn new() -> Self {
        // 加载配置
        let config = ConfigLoader::get_global().expect("全局配置单例未初始化");
        
        // 创建服务注册中心
        let service_register = service_register_center(&config);

        // 创建用户服务的延迟初始化处理器
        let config_clone1 = config.clone();
        let user_service = LazyServiceHandler::new(move || {
            let rt = tokio::runtime::Handle::current();
            let config_clone = config_clone1.clone();
            let client = rt.block_on(async {
                get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(&config_clone, "user-service".to_string()).await
            }).map(|client| UserServiceGrpcClient::new(client)).expect("无法连接用户服务");
            
            UserServiceHandler::new(client)
        });
        
        // 创建好友服务的延迟初始化处理器
        let config_clone2 = config.clone();
        let friend_service = LazyServiceHandler::new(move || {
            let rt = tokio::runtime::Handle::current();
            let config_clone = config_clone2.clone();
            let client = rt.block_on(async {
                get_rpc_client::<FriendServiceClient<LbWithServiceDiscovery>>(&config_clone, "friend-service".to_string()).await
            }).map(|client| FriendServiceGrpcClient::new(client)).expect("无法连接好友服务");
            
            FriendServiceHandler::new(client)
        });
        
        // 创建群组服务的延迟初始化处理器
        let config_clone3 = config.clone();
        let group_service = LazyServiceHandler::new(move || {
            let rt = tokio::runtime::Handle::current();
            let config_clone = config_clone3.clone();
            let client = rt.block_on(async {
                get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(&config_clone, "group-service".to_string()).await
            }).map(|client| GroupServiceGrpcClient::new(client)).expect("无法连接群组服务");
            
            GroupServiceHandler::new(client)
        });

        Self {
            config,
            service_register,
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
                "users" => {
                    // 延迟初始化获取用户服务处理器
                    let mut user_service = self_clone.user_service.get();
                    user_service.handle_request(&method, &path, body).await
                        .unwrap_or_else(|err| {
                            error!("处理用户服务请求失败: {}", err);
                            error_response(&format!("处理用户服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                        })
                },
                "friends" => {
                    // 延迟初始化获取好友服务处理器
                    let mut friend_service = self_clone.friend_service.get();
                    friend_service.handle_request(&method, &path, body).await
                        .unwrap_or_else(|err| {
                            error!("处理好友服务请求失败: {}", err);
                            error_response(&format!("处理好友服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                        })
                },
                "groups" => {
                    // 延迟初始化获取群组服务处理器
                    let mut group_service = self_clone.group_service.get();
                    group_service.handle_request(&method, &path, body).await
                        .unwrap_or_else(|err| {
                            error!("处理群组服务请求失败: {}", err);
                            error_response(&format!("处理群组服务请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                        })
                },
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
        let service_register = self.service_register.clone();
        let service_name = "user-service".to_string();

        Box::pin(async move {
            // 简单的健康检查：尝试从服务注册中心查询用户服务
            match service_register.find_by_name(&service_name).await {
                Ok(services) => !services.is_empty(),
                Err(_) => false,
            }
        })
    }
}

/// 克隆实现
impl Clone for GrpcClientFactoryImpl {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            service_register: self.service_register.clone(),
            user_service: self.user_service.clone(),
            friend_service: self.friend_service.clone(),
            group_service: self.group_service.clone(),
        }
    }
}
