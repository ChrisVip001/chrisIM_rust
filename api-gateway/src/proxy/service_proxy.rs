use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use runtime::configs::routes_config::ServiceType;
use std::sync::Arc;
use tracing::{error, info, warn};
use crate::proxy::services::common::error_response;
use runtime::config::AppConfig;
use contract::Error;
use crate::proxy::grpc_clients::{FriendServiceGrpcClient, GroupServiceGrpcClient, UserServiceGrpcClient};
use runtime::grpc_client::base::get_rpc_client;
use runtime::grpc_client::base::service_names as sn;
use contract::proto::friend::friend_service_client::FriendServiceClient;
use contract::proto::group::group_service_client::GroupServiceClient;
use contract::proto::user::user_service_client::UserServiceClient;
use runtime::service_discovery::LbWithServiceDiscovery;
use crate::proxy::extract_request_body;
use crate::proxy::services::{ChatServiceHandler, CommonServiceHandler, FriendServiceHandler, GroupServiceHandler, UserServiceHandler};
use tokio::sync::{Mutex, RwLock};
use contract::proto::message::chat_service_client::ChatServiceClient;

/// 服务客户端连接池
struct ServiceClients {
    user_client: Option<UserServiceGrpcClient>,
    friend_client: Option<FriendServiceGrpcClient>,
    group_client: Option<GroupServiceGrpcClient>,
    chat_client: Option<ChatServiceClient<LbWithServiceDiscovery>>,
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
    // 串行化连接池重建，避免并发请求触发多次重建
    rebuild_lock: Arc<Mutex<()>>,
}

impl ServiceProxy {
    /// 创建新的服务代理
    ///
    /// 配置由调用方注入，不再回读全局单例，
    /// 这样本结构可以在测试里用一份临时配置构造。
    pub async fn new(config: Arc<AppConfig>) -> Self {
        let proxy = Self {
            config,
            clients: Arc::new(RwLock::new(ServiceClients::new())),
            rebuild_lock: Arc::new(Mutex::new(())),
        };

        // 启动时初始化服务客户端；个别失败不阻塞启动，
        // 后续请求发现客户端缺失时会自动重建（见 get_or_rebuild）
        if let Err(e) = proxy.initialize_clients().await {
            warn!("启动时初始化服务客户端失败（将在请求时自动重试）: {}", e);
        }
        proxy
    }

    /// 获取客户端；缺失时尝试重建连接池
    ///
    /// 此前初始化只有 OnceCell 一次机会，后端服务不可用时客户端
    /// 永远为 None，对应路由持续 503 直到进程重启。
    /// 现在请求路径发现缺失即按需重建，重建期间并发请求串行等待。
    async fn get_or_rebuild<T>(&self, extract: impl Fn(&ServiceClients) -> Option<T>) -> Option<T> {
        if let Some(client) = extract(&*self.clients.read().await) {
            return Some(client);
        }

        let _guard = self.rebuild_lock.lock().await;
        // 双检：等待期间可能已被其他请求重建
        if let Some(client) = extract(&*self.clients.read().await) {
            return Some(client);
        }

        warn!("服务客户端缺失，尝试重建连接池...");
        if let Err(e) = self.initialize_clients().await {
            error!("重建服务客户端连接池失败: {}", e);
            return None;
        }
        extract(&*self.clients.read().await)
    }

    /// 异步初始化所有服务客户端
    async fn initialize_clients(&self) -> Result<(), Error> {
        info!("开始异步初始化服务客户端连接池...");
        
        // 并行创建所有客户端连接
        let user_task = async {
            get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(&self.config, sn::RELATION.to_string())
                .await
                .map(|client| UserServiceGrpcClient::new(client))
        };

        let friend_task = async {
            get_rpc_client::<FriendServiceClient<LbWithServiceDiscovery>>(&self.config, sn::RELATION.to_string())
                .await
                .map(|client| FriendServiceGrpcClient::new(client))
        };

        let group_task = async {
            get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(&self.config, sn::RELATION.to_string())
                .await
                .map(|client| GroupServiceGrpcClient::new(client))
        };

        let chat_task = async {
            get_rpc_client::<ChatServiceClient<LbWithServiceDiscovery>>(&self.config, sn::CHAT.to_string())
                .await
        };

        let (user_result, friend_result, group_result,  chat_result) = tokio::join!(user_task, friend_task, group_task, chat_task);

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
        
        match chat_result {
            Ok(client) => {
                clients.chat_client = Some(client);
                info!("聊天服务客户端初始化成功");
            }
            Err(e) => warn!("聊天服务客户端初始化失败: {}", e),
        }

        info!("服务客户端连接池初始化完成");
        Ok(())
    }

    /// 获取用户服务客户端
    async fn get_user_client(&self) -> Option<UserServiceGrpcClient> {
        self.get_or_rebuild(|c| c.user_client.clone()).await
    }

    /// 获取好友服务客户端
    async fn get_friend_client(&self) -> Option<FriendServiceGrpcClient> {
        self.get_or_rebuild(|c| c.friend_client.clone()).await
    }

    /// 获取群组服务客户端
    async fn get_group_client(&self) -> Option<GroupServiceGrpcClient> {
        self.get_or_rebuild(|c| c.group_client.clone()).await
    }

    /// 获取chat服务客户端
    async fn get_chat_client(&self) -> Option<ChatServiceClient<LbWithServiceDiscovery>> {
        self.get_or_rebuild(|c| c.chat_client.clone()).await
    }

    /// 转发请求到后端服务
    pub async fn forward_request(
        &self,
        req: Request<Body>,
        service_type: &ServiceType,
    ) -> Result<Response<Body>, anyhow::Error> {
        // 提取请求信息
        let (method, path, body, user_info) = match extract_request_body(req).await {
            Ok(data) => data,
            Err(err) => {
                error!("请求解析失败: {}", err);
                return Ok(error_response(&format!("请求解析失败: {}", err), StatusCode::BAD_REQUEST));
            }
        };

        // 根据服务类型决定转发方式
        match service_type {
            // 核心业务服务使用gRPC转发
            ServiceType::User => {
                if let Some(user_client) = self.get_user_client().await {
                    UserServiceHandler::new(user_client).handle_request(&method, &path, body, user_info).await
                } else {
                    warn!("用户服务客户端不可用");
                    Ok(error_response(&format!("服务暂时不可用: {}","user"),  StatusCode::SERVICE_UNAVAILABLE))
                }
            }
            ServiceType::Friend => {
                if let Some(friend_client) = self.get_friend_client().await {
                    FriendServiceHandler::new(friend_client).handle_request(&method, &path, body, user_info).await
                } else {
                    warn!("好友服务客户端不可用");
                    Ok(error_response(&format!("服务暂时不可用: {}","friend"),  StatusCode::SERVICE_UNAVAILABLE))
                }
            }
            ServiceType::Group => {
                if let Some(group_client) = self.get_group_client().await {
                    GroupServiceHandler::new(group_client).handle_request(&method, &path, body, user_info).await
                } else {
                    warn!("群组服务客户端不可用");
                    Ok(error_response(&format!("服务暂时不可用: {}", "group"), StatusCode::SERVICE_UNAVAILABLE))
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
                        .handle_request(&method, &path, body, user_info).await
                } else {
                    warn!("通用服务所需的客户端不完整");
                    Ok(error_response(&format!("通用服务所需的客户端不完整: {}", "common"), StatusCode::SERVICE_UNAVAILABLE))
                }
            }
            ServiceType::Chat => {
                if let Some(chat_client) = self.get_chat_client().await {
                    ChatServiceHandler::new(chat_client).handle_request(&method, &path, body, user_info).await
                } else {
                    warn!("聊天服务客户端不可用");
                    Ok(error_response(&format!("服务暂时不可用: {}", "chat"), StatusCode::SERVICE_UNAVAILABLE))
                }
            }
            _ => {
                Ok(error_response("无效的服务类型", StatusCode::BAD_REQUEST))
            }
        }
    }
}
