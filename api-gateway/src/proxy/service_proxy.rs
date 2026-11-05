use axum::{
    body::Body,
    http::{Request, Response},
};
use runtime::configs::routes_config::ServiceType;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use runtime::config::AppConfig;
use contract::Error;
use crate::error::GatewayError;
use crate::proxy::grpc_clients::{
    ChatServiceGrpcClient, FriendServiceGrpcClient, GroupServiceGrpcClient, UserServiceGrpcClient,
};

use super::extract_request_body;
use super::services::ChatServiceHandler;
use super::services::CommonServiceHandler;
use super::services::FriendServiceHandler;
use super::services::GroupServiceHandler;
use super::services::UserServiceHandler;
use contract::proto::message::chat_service_client::ChatServiceClient;
use contract::proto::friend::friend_service_client::FriendServiceClient;
use contract::proto::group::group_service_client::GroupServiceClient;
use contract::proto::user::user_service_client::UserServiceClient;
use runtime::grpc_client::base::get_rpc_client;
use runtime::grpc_client::base::service_names as sn;
use runtime::service_discovery::LbWithServiceDiscovery;

/// 需要单独重建的下游客户端
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ClientKind {
    User,
    Friend,
    Group,
    Chat,
}

/// 重建退避门：冷却期内不再打 Consul
#[derive(Debug, Default, Clone, Copy)]
struct RetryGate {
    /// 早于这个时刻不再尝试重建
    not_before: Option<Instant>,
    /// 连续失败次数，用于指数退避
    failures: u32,
}

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
    config: Arc<AppConfig>,
    clients: Arc<RwLock<ServiceClients>>,
    /// 每个下游一把锁：只重建缺失的那一个，别的服务不受影响
    rebuild_locks: Arc<HashMap<ClientKind, Arc<Mutex<()>>>>,
    /// 重建失败后的冷却/退避状态
    gates: Arc<Mutex<HashMap<ClientKind, RetryGate>>>,
}

impl ServiceProxy {
    /// 创建新的服务代理
    pub async fn new(config: Arc<AppConfig>) -> Self {
        let rebuild_locks = [
            ClientKind::User,
            ClientKind::Friend,
            ClientKind::Group,
            ClientKind::Chat,
        ]
        .into_iter()
        .map(|kind| (kind, Arc::new(Mutex::new(()))))
        .collect();

        let proxy = Self {
            config,
            clients: Arc::new(RwLock::new(ServiceClients::new())),
            rebuild_locks: Arc::new(rebuild_locks),
            gates: Arc::new(Mutex::new(HashMap::new())),
        };

        // 启动预热：四个下游并行建连，个别失败不阻塞启动
        let (user, friend, group, chat) = tokio::join!(
            proxy.get_user_client(true),
            proxy.get_friend_client(true),
            proxy.get_group_client(true),
            proxy.get_chat_client(true),
        );
        let ready = [user.is_some(), friend.is_some(), group.is_some(), chat.is_some()]
            .iter()
            .filter(|ok| **ok)
            .count();
        info!("服务客户端预热完成: {}/4 可用", ready);
        proxy
    }

    /// 取客户端；缺失时按需重建
    ///
    /// 1. 只重建缺失的那一个，已有连接保持不变；
    /// 2. 重建失败后进入指数退避（1s→30s），冷却期内直接返回 `None`；
    /// 3. 同一个下游同时只有一个重建在跑（按 `ClientKind` 加锁），
    ///    排队期间若已被别的请求重建成功则直接复用。
    async fn obtain<T, Peek, Store>(
        &self,
        kind: ClientKind,
        force: bool,
        peek: Peek,
        build: impl Future<Output = Result<T, Error>>,
        store: Store,
    ) -> Option<T>
    where
        T: Clone,
        Peek: Fn(&ServiceClients) -> Option<T>,
        Store: Fn(&mut ServiceClients, T),
    {
        if let Some(client) = peek(&*self.clients.read().await) {
            return Some(client);
        }

        if !force {
            let gates = self.gates.lock().await;
            if let Some(gate) = gates.get(&kind) {
                if gate.not_before.is_some_and(|t| Instant::now() < t) {
                    // 冷却中：快速失败，别再打 Consul
                    return None;
                }
            }
        }

        let Some(lock) = self.rebuild_locks.get(&kind).cloned() else {
            return None;
        };
        let _guard = lock.lock().await;

        // 双检：排队期间可能已被别的请求重建成功
        if let Some(client) = peek(&*self.clients.read().await) {
            return Some(client);
        }

        debug!(?kind, "重建下游客户端");
        match build.await {
            Ok(client) => {
                store(&mut *self.clients.write().await, client);
                self.gates.lock().await.remove(&kind);
                peek(&*self.clients.read().await)
            }
            Err(e) => {
                let mut gates = self.gates.lock().await;
                let gate = gates.entry(kind).or_default();
                gate.failures = gate.failures.saturating_add(1);
                let backoff = Duration::from_secs(1u64 << gate.failures.min(5)).min(Duration::from_secs(30));
                gate.not_before = Some(Instant::now() + backoff);
                warn!(
                    ?kind,
                    failures = gate.failures,
                    backoff_secs = backoff.as_secs(),
                    "下游客户端重建失败，进入冷却: {}",
                    e
                );
                None
            }
        }
    }

    async fn get_user_client(&self, force: bool) -> Option<UserServiceGrpcClient> {
        self.obtain(
            ClientKind::User,
            force,
            |c| c.user_client.clone(),
            async {
                get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(
                    &self.config,
                    sn::RELATION.to_string(),
                )
                .await
                .map(UserServiceGrpcClient::new)
            },
            |c, client| c.user_client = Some(client),
        )
        .await
    }

    async fn get_friend_client(&self, force: bool) -> Option<FriendServiceGrpcClient> {
        self.obtain(
            ClientKind::Friend,
            force,
            |c| c.friend_client.clone(),
            async {
                get_rpc_client::<FriendServiceClient<LbWithServiceDiscovery>>(
                    &self.config,
                    sn::RELATION.to_string(),
                )
                .await
                .map(FriendServiceGrpcClient::new)
            },
            |c, client| c.friend_client = Some(client),
        )
        .await
    }

    async fn get_group_client(&self, force: bool) -> Option<GroupServiceGrpcClient> {
        self.obtain(
            ClientKind::Group,
            force,
            |c| c.group_client.clone(),
            async {
                get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(
                    &self.config,
                    sn::RELATION.to_string(),
                )
                .await
                .map(GroupServiceGrpcClient::new)
            },
            |c, client| c.group_client = Some(client),
        )
        .await
    }

    async fn get_chat_client(&self, force: bool) -> Option<ChatServiceGrpcClient> {
        self.obtain(
            ClientKind::Chat,
            force,
            |c| c.chat_client.clone(),
            async {
                get_rpc_client::<ChatServiceClient<LbWithServiceDiscovery>>(
                    &self.config,
                    sn::CHAT.to_string(),
                )
                .await
                .map(ChatServiceGrpcClient::new)
            },
            |c, client| c.chat_client = Some(client),
        )
        .await
    }

    /// 转发请求到后端服务
    ///
    /// 返回的 [`GatewayError`] 带语义：参数问题 400、下游不可用 503、
    /// 下游报错 502、下游超时 504。
    pub async fn forward_request(
        &self,
        req: Request<Body>,
        service_type: &ServiceType,
    ) -> Result<Response<Body>, GatewayError> {
        // 提取请求信息
        let (method, path, body, user_info) = extract_request_body(req).await?;

        // 根据服务类型决定转发方式
        match service_type {
            // 核心业务服务使用gRPC转发
            ServiceType::User => {
                let Some(user_client) = self.get_user_client(false).await else {
                    return Err(GatewayError::unavailable("用户服务客户端未就绪"));
                };
                UserServiceHandler::new(user_client)
                    .handle_request(&method, &path, body, user_info)
                    .await
            }
            ServiceType::Friend => {
                let Some(friend_client) = self.get_friend_client(false).await else {
                    return Err(GatewayError::unavailable("好友服务客户端未就绪"));
                };
                FriendServiceHandler::new(friend_client)
                    .handle_request(&method, &path, body, user_info)
                    .await
            }
            ServiceType::Group => {
                let Some(group_client) = self.get_group_client(false).await else {
                    return Err(GatewayError::unavailable("群组服务客户端未就绪"));
                };
                GroupServiceHandler::new(group_client)
                    .handle_request(&method, &path, body, user_info)
                    .await
            }
            ServiceType::Common => {
                // 通用服务只用到好友与群组两个客户端
                let (friend_client, group_client) = tokio::join!(
                    self.get_friend_client(false),
                    self.get_group_client(false),
                );

                let (Some(friend_client), Some(group_client)) = (friend_client, group_client)
                else {
                    return Err(GatewayError::unavailable("通用服务所需的下游客户端不完整"));
                };
                CommonServiceHandler::new(friend_client, group_client)
                    .handle_request(&method, &path, body, user_info)
                    .await
            }
            ServiceType::Chat => {
                let Some(chat_client) = self.get_chat_client(false).await else {
                    return Err(GatewayError::unavailable("聊天服务客户端未就绪"));
                };
                ChatServiceHandler::new(chat_client)
                    .handle_request(&method, &path, body, user_info)
                    .await
            }
            _ => Err(GatewayError::bad_request("无效的服务类型")),
        }
    }
}

use std::future::Future;
