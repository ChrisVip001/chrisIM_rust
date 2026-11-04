use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::mapref::entry::Entry;
use dashmap::mapref::one::Ref;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use cache::Cache;
use runtime::config::AppConfig;
use contract::error::Error;
use contract::proto::message::chat_service_client::ChatServiceClient;
use contract::proto::message::{
    ContentType, GroupMemSeq, Msg, MsgType, PlatformType, SendMsgRequest,
};
use runtime::service_discovery::LbWithServiceDiscovery;
use contract::types::msg::MsgType2;

use crate::push_payload::PushPayload;

pub(crate) use crate::client::Client;

/// 用户ID类型别名
type UserID = String;

/// 连接中心中登记的一条连接
///
/// `conn_id` 是连接的唯一标识，用于区分「同一用户同一平台」先后建立的两条连接。
/// 没有它的话，被顶下线的旧连接在收尾时会把它后继者的登记项一起删掉。
#[derive(Debug)]
pub(crate) struct Conn<C> {
    pub(crate) conn_id: u64,
    pub(crate) client: C,
}

/// 客户端连接中心
///
/// 外层按用户、内层按平台组织，天然支持同一账号多端同时在线，
/// 且读写不需要全局锁。泛型参数让连接中心的调度逻辑可以脱离
/// WebSocket 连接本身被单独测试。
#[derive(Debug)]
pub(crate) struct Hub<C> {
    inner: DashMap<UserID, DashMap<PlatformType, Conn<C>>>,
    next_conn_id: AtomicU64,
}

impl<C> Default for Hub<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> Hub<C> {
    pub(crate) fn new() -> Self {
        Self {
            inner: DashMap::new(),
            next_conn_id: AtomicU64::new(1),
        }
    }

    /// 注册连接，返回 `(新连接ID, 被顶掉的同平台旧连接)`
    ///
    /// 旧连接由调用方决定如何处置（网关的选择是发踢下线信号），
    /// 连接中心本身不感知业务语义。
    pub(crate) fn register(
        &self,
        user_id: &str,
        platform: PlatformType,
        client: C,
    ) -> (u64, Option<C>) {
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let platforms = self.inner.entry(user_id.to_string()).or_default();
        let evicted = platforms
            .insert(platform, Conn { conn_id, client })
            .map(|old| old.client);
        (conn_id, evicted)
    }

    /// 注销连接
    ///
    /// 只有 `conn_id` 与当前登记项一致时才真正移除：旧连接的收尾逻辑
    /// 不会误删同平台的新连接。返回是否真的移除了登记项。
    pub(crate) fn unregister(&self, user_id: &str, platform: PlatformType, conn_id: u64) -> bool {
        match self.inner.entry(user_id.to_string()) {
            Entry::Occupied(mut entry) => {
                let (is_current, empty) = {
                    let platforms = entry.get_mut();
                    let is_current = platforms
                        .get(&platform)
                        .map(|conn| conn.conn_id == conn_id)
                        .unwrap_or(false);
                    if is_current {
                        platforms.remove(&platform);
                    }
                    (is_current, platforms.is_empty())
                };

                if empty {
                    let _ = entry.remove();
                }
                is_current
            }
            Entry::Vacant(_) => false,
        }
    }

    /// 获取某个用户全部平台的连接
    ///
    /// 返回的 `Ref` 持有该用户所在分片的读锁，**不要跨 await 长期持有**，
    /// 也不要持有期间调用 `register`/`unregister`（会自死锁）。
    /// 只查连接ID时请用 [`Hub::conn_id_of`]。
    pub(crate) fn get(&self, user_id: &str) -> Option<Ref<'_, UserID, DashMap<PlatformType, Conn<C>>>> {
        self.inner.get(user_id)
    }

    /// 查询某用户在某平台当前登记的连接ID
    #[cfg(test)]
    pub(crate) fn conn_id_of(&self, user_id: &str, platform: PlatformType) -> Option<u64> {
        let platforms = self.inner.get(user_id)?;
        let conn_id = platforms.get(&platform).map(|conn| conn.conn_id);
        conn_id
    }

    /// 当前在线用户及其平台列表的快照，用于运维排查接口
    pub(crate) fn snapshot(&self) -> Vec<(String, Vec<PlatformType>)> {
        self.inner
            .iter()
            .map(|entry| {
                let platforms = entry.value().iter().map(|conn| *conn.key()).collect();
                (entry.key().clone(), platforms)
            })
            .collect()
    }

    /// 在线用户数（按用户去重，不按连接数）
    #[cfg(test)]
    pub(crate) fn user_count(&self) -> usize {
        self.inner.len()
    }
}

/// 连接管理器
///
/// 负责管理所有WebSocket客户端连接的生命周期，包括：
/// - 客户端注册和注销
/// - 消息路由和分发
/// - 与msg-server的gRPC通信
/// - 缓存操作和序列号管理
#[derive(Clone)]
pub struct Manager {
    /// 消息广播发送器
    /// 用于将客户端发送的消息转发到处理队列
    tx: mpsc::Sender<Msg>,

    /// 客户端连接中心
    hub: Arc<Hub<Client>>,

    /// 缓存接口
    /// 用于序列号管理和用户状态缓存
    pub cache: Arc<dyn Cache>,

    /// 聊天服务RPC客户端
    /// 用于与msg-server通信，发送消息到Kafka队列
    pub chat_rpc: ChatServiceClient<LbWithServiceDiscovery>,
}

#[allow(dead_code)]
impl Manager {
    /// 创建新的连接管理器实例
    pub async fn new(tx: mpsc::Sender<Msg>, config: &AppConfig) -> Self {
        let cache = cache::cache(config).await;

        let chat_rpc = runtime::grpc_client::base::get_rpc_client(config, config.rpc.chat.name.clone())
            .await
            .expect("无法连接到聊天RPC服务");

        Manager {
            tx,
            hub: Arc::new(Hub::new()),
            cache,
            chat_rpc,
        }
    }

    /// 注册客户端连接，返回连接ID供注销时校验
    ///
    /// 同一用户在同一平台重复登录时，旧连接会收到踢下线信号并主动关闭，
    /// 新连接直接覆盖登记项。这里不再等待固定时长，
    /// 因为旧连接收尾时不会再误删新连接。
    pub async fn register(&mut self, id: String, client: Client) -> u64 {
        let platform = client.platform;
        let (conn_id, evicted) = self.hub.register(&id, platform, client);

        if let Some(old) = evicted {
            info!(
                "检测到用户 {} 在平台 {:?} 的重复连接，向旧连接发送踢下线信号",
                id, platform
            );
            if let Err(e) = old.notify_sender.send(()).await {
                warn!(
                    "向用户 {} 平台 {:?} 的旧连接发送踢下线信号失败: {}",
                    id, platform, e
                );
            }
        }

        info!(
            "用户 {} 在平台 {:?} 的连接已注册成功，连接ID={}",
            id, platform, conn_id
        );
        conn_id
    }

    /// 注销客户端连接
    ///
    /// 只有连接ID匹配时才会移除；用户的所有平台都断开后自动清理用户条目。
    pub async fn unregister(&mut self, id: String, platform: PlatformType, conn_id: u64) {
        if self.hub.unregister(&id, platform, conn_id) {
            debug!("注销客户端连接: user={}, platform={:?}, conn={}", id, platform, conn_id);
        } else {
            debug!(
                "跳过注销：该连接已被同平台的新连接替换 (user={}, conn={})",
                id, conn_id
            );
        }
    }

    /// 在线用户快照
    pub fn online_users(&self) -> Vec<(String, Vec<PlatformType>)> {
        self.hub.snapshot()
    }

    /// 发送群聊消息
    ///
    /// 处理群聊消息的分发逻辑：
    /// 1. 向发送者的其他平台发送消息副本
    /// 2. 为每个群成员分配正确的序列号
    /// 3. 向所有在线的群成员推送消息
    pub async fn send_group(&self, obj_ids: Vec<GroupMemSeq>, mut msg: Msg) {
        // 向发送者的其他平台发送消息副本
        self.send_to_self(&msg.send_id, &msg).await;

        // 将发送序列号设置为0，因为这是推送给接收者的消息
        msg.send_seq = 0;

        // 为每个群成员发送消息
        for mem in obj_ids {
            if let Some(clients) = self.hub.get(&mem.mem_id) {
                // 为每个成员设置正确的接收序列号
                msg.seq = mem.cur_seq;
                // 为每个成员设置正确的接收者id
                msg.receiver_id = mem.mem_id.clone();

                // 向该成员的所有在线客户端发送消息
                self.send_msg_to_clients(&clients, &msg).await;
            }
        }
    }

    /// 向发送者的其他平台发送消息副本
    ///
    /// 当用户在多个平台同时在线时，需要向发送者的其他平台
    /// 发送消息副本，以保持消息同步。
    async fn send_to_self(&self, id: &str, msg: &Msg) {
        if let Some(clients) = self.hub.get(id) {
            let content = match serde_json::to_string(&self.build_push_payload(msg, true).await) {
                Ok(content) => content,
                Err(e) => {
                    error!("消息JSON序列化失败: {}", e);
                    return;
                }
            };

            let sent = Self::send_text_to_all(&clients, &content).await;
            if sent > 0 {
                debug!("成功向发送者的 {} 个其他平台发送消息副本", sent);
            }
        }
    }

    /// 发送单聊消息
    ///
    /// 处理单聊消息的分发逻辑：
    /// 1. 向接收者的所有在线客户端发送消息
    /// 2. 向发送者的其他平台发送消息副本
    pub async fn send_single_msg(&self, obj_id: &str, msg: &Msg) {
        // 向接收者发送消息
        if let Some(clients) = self.hub.get(obj_id) {
            self.send_msg_to_clients(&clients, msg).await;
        }

        // 向发送者的其他平台发送消息副本
        if msg.msg_type != MsgType::FriendApplyReq as i32 {
            self.send_to_self(&msg.send_id, msg).await;
        }
    }

    /// 向客户端连接发送消息
    async fn send_msg_to_clients(
        &self,
        clients: &DashMap<PlatformType, Conn<Client>>,
        msg: &Msg,
    ) {
        if clients.is_empty() {
            error!("未找到客户端连接");
            return;
        }

        let content = match serde_json::to_string(&self.build_push_payload(msg, false).await) {
            Ok(content) => content,
            Err(e) => {
                error!("消息JSON序列化失败: {}", e);
                return;
            }
        };

        let sent = Self::send_text_to_all(clients, &content).await;
        if sent == 0 {
            error!("未成功发送消息给任何客户端");
        } else {
            debug!("成功向 {} 个客户端发送消息", sent);
        }
    }

    /// 向同一用户的全部平台连接发送同一份报文，返回成功数
    async fn send_text_to_all(
        clients: &DashMap<PlatformType, Conn<Client>>,
        content: &str,
    ) -> usize {
        let mut sent = 0;
        for entry in clients.iter() {
            let (platform, conn) = (entry.key(), entry.value());
            if let Err(e) = conn.client.send_text(content.to_string()).await {
                error!("向平台 {:?} 发送消息失败: {}", platform, e);
            } else {
                sent += 1;
            }
        }
        sent
    }

    /// 组装下行推送报文
    ///
    /// 会话ID的取法在两种视角下不同：
    /// - 发给自己的副本：会话对端是消息的接收者
    /// - 发给接收者的消息：会话对端是消息的发送者
    async fn build_push_payload(&self, msg: &Msg, is_self: bool) -> PushPayload {
        let mt = MsgType::try_from(msg.msg_type).unwrap_or(MsgType::SingleMsg);
        let mt2 = MsgType2::from(mt);

        let conversation_id = match mt2 {
            MsgType2::Friend | MsgType2::System => {
                if is_self {
                    msg.receiver_id.clone()
                } else {
                    msg.send_id.clone()
                }
            }
            MsgType2::Group => msg.group_id.clone(),
        };

        // 发给自己的副本不计入未读
        let unread_count = self
            .calculate_and_update_unread_count(msg, &conversation_id, is_self)
            .await;

        PushPayload::single(
            msg.clone(),
            conversation_id,
            MsgType2::mt2_str(mt2),
            unread_count,
            is_self,
        )
    }

    /// 运行消息处理循环
    ///
    /// 这是Manager的主要工作方法，持续监听消息队列：
    /// 1. 接收来自WebSocket客户端的消息
    /// 2. 处理消息（增加序列号、发送到msg-server）
    /// 3. 向发送者返回处理结果
    pub async fn run(&mut self, mut receiver: mpsc::Receiver<Msg>) {
        info!("连接管理器已启动");

        while let Some(mut message) = receiver.recv().await {
            self.process_message(&mut message).await;
            debug!("消息已处理并发送到Kafka: {:?}", message);
        }
    }

    /// 处理单条消息
    async fn process_message(&mut self, message: &mut Msg) {
        // 判断是否为心跳消息
        if message.msg_type == MsgType::Heartbeat as i32
            || message.msg_type == MsgType::MsgRecResp as i32
        {
            debug!("收到心跳消息，直接响应: user_id={}", message.send_id);
            message.msg_type = MsgType::MsgRecResp as i32;
            message.content.clear();
            return;
        }

        match self.send_rpc_message(message.clone()).await {
            Ok(response) => {
                debug!("消息发送成功");
                *message = response;
                message.content.clear();
                message.msg_type = MsgType::MsgRecResp as i32;
            }
            Err(err) => {
                error!("gRPC调用失败: {:?}", err);
                self.create_error_message(message, err);
            }
        }
    }

    /// 通过gRPC发送消息到msg-server
    async fn send_rpc_message(&self, message: Msg) -> Result<Msg, tonic::Status> {
        let mut chat_rpc = self.chat_rpc.clone();
        chat_rpc
            .send_msg(SendMsgRequest {
                message: Some(message),
            })
            .await
            .map(|res| res.into_inner())
    }

    /// 创建错误消息
    fn create_error_message(&self, message: &mut Msg, error: impl ToString) {
        message.content_type = ContentType::Error as i32;
        message.msg_type = MsgType::MsgRecResp as i32;
        message.content = error.to_string().into_bytes();
    }

    /// 广播消息
    ///
    /// 将消息发送到处理队列，由run方法的循环处理。
    pub async fn broadcast(&self, msg: Msg) -> Result<(), Error> {
        self.tx
            .send(msg)
            .await
            .map_err(|e| Error::BroadCastError(e.to_string()))
    }

    /// 计算并更新未读数
    async fn calculate_and_update_unread_count(
        &self,
        msg: &Msg,
        conversation_id: &str,
        is_self: bool,
    ) -> i32 {
        // 如果是发送给自己的消息副本，未读数为0，无需查询和更新缓存
        if is_self {
            return 0;
        }

        let user_id = &msg.receiver_id;

        let old_unread = self
            .cache
            .unread_count_sum(user_id, conversation_id)
            .await
            .unwrap_or(0);

        let unread_count = if msg.is_read {
            old_unread
        } else {
            old_unread + 1
        };

        let _ = self
            .cache
            .unread_count_set(user_id, conversation_id, &unread_count)
            .await;

        unread_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用连接：只需要能区分「是哪一条连接」
    #[derive(Debug, PartialEq)]
    struct FakeClient(&'static str);

    fn hub() -> Hub<FakeClient> {
        Hub::new()
    }

    #[tokio::test]
    async fn register_keeps_all_platforms_of_same_user() {
        let hub = hub();

        let (pc_conn, evicted) = hub.register("u1", PlatformType::Pc, FakeClient("pc"));
        assert_eq!(evicted, None);

        let (mobile_conn, evicted) = hub.register("u1", PlatformType::Mobile, FakeClient("mobile"));
        assert_eq!(evicted, None);

        assert_ne!(pc_conn, mobile_conn);
        assert_eq!(hub.user_count(), 1);
        assert_eq!(hub.get("u1").map(|c| c.len()), Some(2));
    }

    #[tokio::test]
    async fn same_platform_login_evicts_previous_connection() {
        let hub = hub();

        let (first_conn, _) = hub.register("u1", PlatformType::Pc, FakeClient("pc-old"));
        let (second_conn, evicted) = hub.register("u1", PlatformType::Pc, FakeClient("pc-new"));

        // 旧连接被交还给调用方去发踢下线信号，新连接获得新的连接ID
        assert_eq!(evicted, Some(FakeClient("pc-old")));
        assert_ne!(first_conn, second_conn);

        // 同一平台只保留一条连接，指向最新的那次登录
        let clients = hub.get("u1").expect("用户条目应存在");
        assert_eq!(clients.len(), 1);
        assert_eq!(clients.get(&PlatformType::Pc).unwrap().conn_id, second_conn);
    }

    #[tokio::test]
    async fn stale_connection_cleanup_keeps_new_connection() {
        let hub = hub();

        let (first_conn, _) = hub.register("u1", PlatformType::Pc, FakeClient("pc-old"));
        let (second_conn, _) = hub.register("u1", PlatformType::Pc, FakeClient("pc-new"));

        // 旧连接的收尾逻辑晚于新连接注册才执行
        let removed = hub.unregister("u1", PlatformType::Pc, first_conn);
        assert!(!removed, "旧连接不应该能删掉新连接的登记项");

        assert_eq!(
            hub.conn_id_of("u1", PlatformType::Pc),
            Some(second_conn),
            "新连接必须仍然在线"
        );

        // 新连接自己注销时才真正移除
        assert!(hub.unregister("u1", PlatformType::Pc, second_conn));
        assert_eq!(hub.user_count(), 0);
    }

    #[tokio::test]
    async fn unregister_one_platform_keeps_others() {
        let hub = hub();

        let (pc_conn, _) = hub.register("u1", PlatformType::Pc, FakeClient("pc"));
        let (mobile_conn, _) = hub.register("u1", PlatformType::Mobile, FakeClient("mobile"));

        assert!(hub.unregister("u1", PlatformType::Pc, pc_conn));

        // 用户仍有移动端在线，用户条目不能被清掉
        assert_eq!(hub.user_count(), 1);
        assert_eq!(hub.conn_id_of("u1", PlatformType::Pc), None);
        assert_eq!(hub.conn_id_of("u1", PlatformType::Mobile), Some(mobile_conn));

        assert!(hub.unregister("u1", PlatformType::Mobile, mobile_conn));
        assert_eq!(hub.user_count(), 0);
    }

    #[tokio::test]
    async fn unregister_unknown_user_is_noop() {
        let hub = hub();
        assert!(!hub.unregister("nobody", PlatformType::Pc, 1));
        assert_eq!(hub.user_count(), 0);
    }

    #[tokio::test]
    async fn snapshot_reports_platforms_per_user() {
        let hub = hub();
        hub.register("u1", PlatformType::Pc, FakeClient("pc"));
        hub.register("u1", PlatformType::Mobile, FakeClient("mobile"));
        hub.register("u2", PlatformType::Web, FakeClient("web"));

        let mut snapshot = hub.snapshot();
        snapshot.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].0, "u1");
        assert_eq!(snapshot[0].1.len(), 2);
        assert_eq!(snapshot[1].0, "u2");
        assert_eq!(snapshot[1].1, vec![PlatformType::Web]);
    }
}
