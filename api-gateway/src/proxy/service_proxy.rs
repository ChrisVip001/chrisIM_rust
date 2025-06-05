use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use common::configs::routes_config::ServiceType;
use std::sync::Arc;
use tracing::{error, info, warn};
use crate::proxy::services::common::error_response;
use common::config::{AppConfig, ConfigLoader};
use common::Error;
use common::grpc_client::{ChatServiceGrpcClient, FriendServiceGrpcClient, GroupServiceGrpcClient, UserServiceGrpcClient};
use common::grpc_client::base::get_rpc_client;
use common::proto::friend::friend_service_client::FriendServiceClient;
use common::proto::group::group_service_client::GroupServiceClient;
use common::proto::user::user_service_client::UserServiceClient;
use common::service_discovery::LbWithServiceDiscovery;
use crate::proxy::extract_request_body;
use crate::proxy::services::{ChatServiceHandler, CommonServiceHandler, FriendServiceHandler, GroupServiceHandler, UserServiceHandler};
use tokio::sync::{OnceCell, RwLock};

/// 服务客户端连接池
struct ServiceClients {
    user_client: Option<UserServiceGrpcClient>,
    friend_client: Option<FriendServiceGrpcClient>,
    group_client: Option<GroupServiceGrpcClient>,
    chat_client: Option<ChatServiceGrpcClient>,
}

impl ServiceClients {
    fn new() -> Self {
        Self {
            user_client: None,
            friend_client: None,
            group_client: None,
            chat_client: None,
        }
    }
}

/// 服务代理 - 负责转发请求到后端服务
#[derive(Clone)]
pub struct ServiceProxy {
    // 应用配置
    config: Arc<AppConfig>,
    // 服务客户端连接池
    clients: Arc<RwLock<ServiceClients>>,
    // 用于确保只初始化一次
    init_once: Arc<OnceCell<()>>,
}

impl ServiceProxy {
    /// 创建新的服务代理
    pub async fn new() -> Self {
        // 加载配置
        let config = ConfigLoader::get_global().expect("全局配置单例未初始化");
        
        let proxy = Self {
            config,
            clients: Arc::new(RwLock::new(ServiceClients::new())),
            init_once: Arc::new(OnceCell::new()),
        };

        // 异步初始化服务客户端
        proxy.ensure_clients_initialized().await;
        proxy
    }

    /// 确保服务客户端已初始化（只初始化一次）
    async fn ensure_clients_initialized(&self) {
        self.init_once
            .get_or_init(|| async {
                if let Err(e) = self.initialize_clients().await {
                    error!("初始化服务客户端失败: {}", e);
                }
            })
            .await;
    }

    /// 异步初始化所有服务客户端
    async fn initialize_clients(&self) -> Result<(), Error> {
        info!("开始异步初始化服务客户端连接池...");
        
        // 并行创建所有客户端连接
        let user_task = async {
            get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(&self.config, "user".to_string())
                .await
                .map(|client| UserServiceGrpcClient::new(client))
        };
        
        let friend_task = async {
            get_rpc_client::<FriendServiceClient<LbWithServiceDiscovery>>(&self.config, "friend".to_string())
                .await
                .map(|client| FriendServiceGrpcClient::new(client))
        };
        
        let group_task = async {
            get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(&self.config, "group".to_string())
                .await
                .map(|client| GroupServiceGrpcClient::new(client))
        };

        let (user_result, friend_result, group_result) = tokio::join!(user_task, friend_task, group_task);

        // 更新客户端连接池
        let mut clients = self.clients.write().await;
        
        match user_result {
            Ok(client) => {
                clients.user_client = Some(client);
                info!("用户服务客户端初始化成功");
            }
            Err(e) => warn!("用户服务客户端初始化失败: {}", e),
        }
        
        match friend_result {
            Ok(client) => {
                clients.friend_client = Some(client);
                info!("好友服务客户端初始化成功");
            }
            Err(e) => warn!("好友服务客户端初始化失败: {}", e),
        }
        
        match group_result {
            Ok(client) => {
                clients.group_client = Some(client);
                info!("群组服务客户端初始化成功");
            }
            Err(e) => warn!("群组服务客户端初始化失败: {}", e),
        }

        info!("服务客户端连接池初始化完成");
        Ok(())
    }

    /// 获取用户服务客户端
    async fn get_user_client(&self) -> Option<UserServiceGrpcClient> {
        self.ensure_clients_initialized().await;
        self.clients.read().await.user_client.clone()
    }

    /// 获取好友服务客户端
    async fn get_friend_client(&self) -> Option<FriendServiceGrpcClient> {
        self.ensure_clients_initialized().await;
        self.clients.read().await.friend_client.clone()
    }

    /// 获取群组服务客户端
    async fn get_group_client(&self) -> Option<GroupServiceGrpcClient> {
        self.ensure_clients_initialized().await;
        self.clients.read().await.group_client.clone()
    }
    
    /// 获取chat服务客户端
    async fn get_chat_client(&self) -> Option<ChatServiceGrpcClient> {
        self.ensure_clients_initialized().await;
        self.clients.read().await.chat_client.clone()
    }

    /// 转发请求到后端服务
    pub async fn forward_request(
        &self,
        req: Request<Body>,
        service_type: &ServiceType,
    ) -> Response<Body> {
        // 提取请求信息
        let (method, path, body, user_info) = match extract_request_body(req).await {
            Ok(data) => data,
            Err(err) => {
                error!("请求解析失败: {}", err);
                return error_response(&format!("请求解析失败: {}", err), StatusCode::BAD_REQUEST);
            }
        };

        // 根据服务类型决定转发方式
        match service_type {
            // 核心业务服务使用gRPC转发
            ServiceType::User => {
                if let Some(user_client) = self.get_user_client().await {
                    UserServiceHandler::new(user_client).handle_request(&method, &path, body, user_info).await.unwrap_or_else(|err| {
                        error!("处理用户服务请求失败: {}", err);
                        error_response(&format!("处理请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    })
                } else {
                    warn!("用户服务客户端不可用");
                    self.service_unavailable_response("user")
                }
            }
            ServiceType::Friend => {
                if let Some(friend_client) = self.get_friend_client().await {
                    FriendServiceHandler::new(friend_client).handle_request(&method, &path, body, user_info).await.unwrap_or_else(|err| {
                        error!("处理好友服务请求失败: {}", err);
                        error_response(&format!("处理请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    })
                } else {
                    warn!("好友服务客户端不可用");
                    self.service_unavailable_response("friend")
                }
            }
            ServiceType::Group => {
                if let Some(group_client) = self.get_group_client().await {
                    GroupServiceHandler::new(group_client).handle_request(&method, &path, body, user_info).await.unwrap_or_else(|err| {
                        error!("处理群组服务请求失败: {}", err);
                        error_response(&format!("处理请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    })
                } else {
                    warn!("群组服务客户端不可用");
                    self.service_unavailable_response("group")
                }
            }
            ServiceType::Common => {
                // 通用服务需要多个客户端
                let user_client = self.get_user_client().await;
                let friend_client = self.get_friend_client().await;
                let group_client = self.get_group_client().await;

                if let (Some(user_client), Some(friend_client), Some(group_client)) = 
                    (user_client, friend_client, group_client) {
                    CommonServiceHandler::new(user_client, friend_client, group_client)
                        .handle_request(&method, &path, body, user_info).await.unwrap_or_else(|err| {
                        error!("处理通用服务请求失败: {}", err);
                        error_response(&format!("处理请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    })
                } else {
                    warn!("通用服务所需的客户端不完整");
                    self.service_unavailable_response("common")
                }
            }
            ServiceType::Chat => {
                if let Some(chat_client) = self.get_chat_client().await {
                    ChatServiceHandler::new(chat_client).handle_request(&method, &path, body, user_info).await.unwrap_or_else(|err| {
                        error!("处理聊天服务请求失败: {}", err);
                        error_response(&format!("处理请求失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                    })
                } else {
                    warn!("聊天服务客户端不可用");
                    self.service_unavailable_response("chat")
                }
            }
            _ => {
                error_response("无效的服务类型", StatusCode::BAD_REQUEST)
            }
        }
    }
    
    /// 服务不可用响应
    fn service_unavailable_response(&self, service_name: &str) -> Response<Body> {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "service_unavailable",
                "message": format!("服务暂时不可用: {}", service_name)
            })),
        )
            .into_response()
    }

    
}
