use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::Utf8Bytes;
use dashmap::mapref::entry::Entry;
use dashmap::mapref::one::Ref;
use dashmap::DashMap;
use futures::stream::{self, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use cache::PresenceCache;
use runtime::config::AppConfig;
use contract::error::Error;
use contract::proto::message::chat_service_client::ChatServiceClient;
use contract::proto::message::{
    ContentType, GroupMemSeq, Msg, MsgType, PlatformType, SendMsgRequest,
};
use runtime::service_discovery::LbWithServiceDiscovery;
use contract::types::msg::MsgType2;

use crate::client::Client;
use crate::push_payload::PushPayload;

/// 群消息并发推送度
///
/// 上限同时约束并发 WebSocket 写入数与同时持有的连接引用数。
/// 报文在进入循环前只序列化一次，循环内不再产生消息副本。
const GROUP_PUSH_CONCURRENCY: usize = 32;

/// 单个上行 worker 的队列容量
///
/// 有界队列提供背压：worker 处理不过来时，消息发送方（连接读循环）
/// 在入队处等待，而不是无限堆积内存。
const UPSTREAM_QUEUE_CAP: usize = 256;

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
/// - 消息路由和分发（上行按会话哈希到多个 worker 并行处理）
/// - 与msg-server的gRPC通信
/// - 缓存操作和序列号管理
///
/// 上行并行模型：所有连接的上行消息按「会话键」哈希到 N 个 worker，
/// 同一会话的消息始终落在同一 worker 上串行处理（保序），
/// 不同会话之间并行。worker 数由 `websocket.upstream_workers` 配置。
#[derive(Clone)]
pub struct Manager {
    /// 上行处理队列组
    ///
    /// 与 worker 一一对应；广播时按会话键哈希选择队列。
    /// 最后一个持有者销毁后队列关闭，对应 worker 随之退出。
    upstream: Vec<mpsc::Sender<Msg>>,

    /// 客户端连接中心
    hub: Arc<Hub<Client>>,

    /// 缓存接口
    /// 用于序列号管理和用户状态缓存
    pub cache: Arc<dyn PresenceCache>,

    /// 聊天服务RPC客户端
    /// 用于与msg-server通信，发送消息到Kafka队列
    pub chat_rpc: ChatServiceClient<LbWithServiceDiscovery>,

    /// 本网关的 gRPC 地址（host:port）
    ///
    /// 写入 Redis 路由表，供 msg-server 定向推送，
    /// 消除「消息广播到全部网关实例」的扇出成本。
    gateway_addr: String,
}

#[allow(dead_code)]
impl Manager {
    /// 创建新的连接管理器实例，并启动上行处理 worker
    pub async fn new(config: &AppConfig) -> Self {
        let cache: Arc<dyn PresenceCache> = cache::cache(config).await;

        let chat_rpc: ChatServiceClient<LbWithServiceDiscovery> =
            runtime::grpc_client::base::get_rpc_client(
                config,
                runtime::grpc_client::base::service_names::CHAT.to_string(),
            )
            .await
            .expect("无法连接到聊天RPC服务");

        let hub = Arc::new(Hub::new());

        // 本网关 gRPC 地址：与 Consul 注册的 host:port 一致，
        // msg-server 内网可达即可直连
        let gateway_addr = format!("{}:{}", config.rpc.ws.host, config.rpc.ws.port);

        // 按配置启动 N 个上行 worker，每个 worker 独占一条有界队列
        let worker_count = config.websocket.worker_count();
        let mut upstream = Vec::with_capacity(worker_count);
        for worker_id in 0..worker_count {
            let (tx, mut rx) = mpsc::channel::<Msg>(UPSTREAM_QUEUE_CAP);
            let worker = Self {
                upstream: Vec::new(), // worker 自身不再分发，避免误用
                hub: hub.clone(),
                cache: cache.clone(),
                chat_rpc: chat_rpc.clone(),
                gateway_addr: gateway_addr.clone(),
            };
            tokio::spawn(async move {
                debug!("上行worker #{} 已启动", worker_id);
                while let Some(mut message) = rx.recv().await {
                    worker.process_message(&mut message).await;
                }
                debug!("上行worker #{} 已退出", worker_id);
            });
            upstream.push(tx);
        }
        info!("上行处理已启动：{} 个 worker，队列容量 {}", worker_count, UPSTREAM_QUEUE_CAP);

        Manager {
            upstream,
            hub,
            cache,
            chat_rpc,
            gateway_addr,
        }
    }

    /// 注册客户端连接，返回连接ID供注销时校验
    ///
    /// 同一用户在同一平台重复登录时，旧连接会收到踢下线信号并主动关闭，
    /// 新连接直接覆盖登记项。这里不再等待固定时长，
    /// 因为旧连接收尾时不会再误删新连接。
    ///
    /// 同时把「用户平台 → 本网关地址」写入 Redis 路由表，
    /// 供 msg-server 定向推送。路由表是尽力而为的旁路数据：
    /// 写入失败只影响推送定向性（退化为广播），不影响连接可用性。
    pub async fn register(&mut self, id: String, client: Client) -> u64 {
        let platform = client.platform;
        let (conn_id, evicted) = self.hub.register(&id, platform, client);

        if let Err(e) = self
            .cache
            .register_user_gateway_route(&id, platform as i32, &self.gateway_addr)
            .await
        {
            warn!("写入用户 {} 平台 {:?} 的网关路由失败（推送将退化为广播）: {}", id, platform, e);
        }

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

            // 只在确实移除了登记项时清理路由；被顶下线的旧连接
            // 不能删掉同平台新连接刚写入的路由
            if let Err(e) = self.cache.unregister_user_gateway_route(&id, platform as i32).await {
                warn!("清理用户 {} 平台 {:?} 的网关路由失败: {}", id, platform, e);
            }
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
    /// 2. 向所有在线的群成员推送消息
    ///
    /// 读扩散下序号是会话级的，所有成员看到同一个 `seq`（见 msg-server
    /// 消费端：`cur_seq = msg.seq`）；群消息的会话标识取自 `group_id`，
    /// 与成员无关。因此推送报文对全部成员完全一致——
    /// **序列化一次**，之后对每个接收者只做 O(1) 的帧克隆
    /// （`Utf8Bytes` 基于 `Bytes`，克隆仅增加引用计数）。
    /// 此前逐成员深拷贝消息体并重复序列化，万人群一条消息要付出
    /// 万次内存分配 + 万次 JSON 编码的代价。
    pub async fn send_group(&self, obj_ids: Vec<GroupMemSeq>, mut msg: Msg) {
        // 向发送者的其他平台发送消息副本（方向标记不同，单独序列化）
        self.send_to_self(&msg.send_id, &msg).await;

        // 将发送序列号设置为0，因为这是推送给接收者的消息
        msg.send_seq = 0;

        // 推送报文对所有成员一致，序列化一次后全员共享
        let frame = match Self::serialize_push(&self.build_push_payload(&msg, false)) {
            Ok(frame) => frame,
            Err(e) => {
                error!("群聊消息JSON序列化失败: {}", e);
                return;
            }
        };

        let hub = self.hub.clone();
        stream::iter(obj_ids)
            .map(|mem| {
                let hub = hub.clone();
                // 引用计数克隆（O(1)），供该成员的全部平台连接共享
                let frame = frame.clone();
                async move {
                    // 不在线的成员直接跳过，不做任何推送准备
                    if let Some(clients) = hub.get(&mem.mem_id) {
                        Self::send_frame_to_all(&clients, &frame).await;
                    }
                }
            })
            .buffer_unordered(GROUP_PUSH_CONCURRENCY)
            .count()
            .await;
    }

    /// 向发送者的其他平台发送消息副本
    ///
    /// 当用户在多个平台同时在线时，需要向发送者的其他平台
    /// 发送消息副本，以保持消息同步。
    async fn send_to_self(&self, id: &str, msg: &Msg) {
        if let Some(clients) = self.hub.get(id) {
            let frame = match Self::serialize_push(&self.build_push_payload(msg, true)) {
                Ok(frame) => frame,
                Err(e) => {
                    error!("消息JSON序列化失败: {}", e);
                    return;
                }
            };

            let sent = Self::send_frame_to_all(&clients, &frame).await;
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
            let frame = match Self::serialize_push(&self.build_push_payload(msg, false)) {
                Ok(frame) => frame,
                Err(e) => {
                    error!("消息JSON序列化失败: {}", e);
                    return;
                }
            };
            Self::send_frame_to_all(&clients, &frame).await;
        }

        // 向发送者的其他平台发送消息副本
        if msg.msg_type != MsgType::FriendApplyReq as i32 {
            self.send_to_self(&msg.send_id, msg).await;
        }
    }

    /// 序列化下行报文为可共享的帧数据
    ///
    /// 返回的 `Utf8Bytes` 基于 `Bytes`，克隆为 O(1)，
    /// 多个接收者共享同一份序列化结果。
    fn serialize_push<T: serde::Serialize>(value: &T) -> Result<Utf8Bytes, serde_json::Error> {
        serde_json::to_string(value).map(Utf8Bytes::from)
    }

    /// 向同一用户的全部平台连接发送同一份帧数据，返回成功数
    async fn send_frame_to_all(
        clients: &DashMap<PlatformType, Conn<Client>>,
        frame: &Utf8Bytes,
    ) -> usize {
        let mut sent = 0;
        for entry in clients.iter() {
            let (platform, conn) = (entry.key(), entry.value());
            // Utf8Bytes 克隆是引用计数操作，不复制报文内容
            if let Err(e) = conn.client.send_frame(frame.clone()).await {
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
    fn build_push_payload(&self, msg: &Msg, is_self: bool) -> PushPayload {
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

        // 读扩散下未读数由 conversation.last_msg_seq - user_conversation.last_read_seq
        // 得出，由会话列表接口统一下发，客户端收到推送后本地自增即可。
        // 推送时若逐成员读写 Redis，万人群一条消息就是 2 万次往返。
        PushPayload::single(
            msg.clone(),
            conversation_id,
            MsgType2::mt2_str(mt2),
            0,
            is_self,
        )
    }

    /// 计算上行消息的路由键
    ///
    /// 与 Kafka 分区键、落库会话序号共用 `contract::types::conversation`
    /// 的唯一推导：同一会话 → 同一 worker（保序），A→B 与 B→A 得到同一键。
    /// 键格式是持久化数据的一部分，此处不做任何二次推导。
    fn route_key(msg: &Msg) -> String {
        contract::types::conversation::conversation_id_of(msg)
    }

    /// 会话键 → worker 序号（确定性哈希，测试可复现）
    fn worker_index(key: &str, worker_count: usize) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % worker_count as u64) as usize
    }

    /// 处理单条消息
    async fn process_message(&self, message: &mut Msg) {
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

        // 处理结果必须回传发送者：MsgRecResp 让客户端确认 local_id 已受理，
        // 失败消息（ContentType::Error）让客户端知道消息没有发出去。
        // 此前结果在这里被直接丢弃，发送失败时客户端永远无感知。
        self.send_result_to_sender(message).await;
    }

    /// 把处理结果原样回传给发送者的全部在线连接
    async fn send_result_to_sender(&self, msg: &Msg) {
        let Some(clients) = self.hub.get(&msg.send_id) else {
            warn!(
                "处理结果无法回传：发送者 {} 已离线, local_id={}, msg_type={}",
                msg.send_id, msg.local_id, msg.msg_type
            );
            return;
        };

        let frame = match Self::serialize_push(msg) {
            Ok(frame) => frame,
            Err(e) => {
                error!("处理结果JSON序列化失败: {}", e);
                return;
            }
        };

        let sent = Self::send_frame_to_all(&clients, &frame).await;
        debug!("处理结果已回传给发送者 {} 的 {} 个连接", msg.send_id, sent);
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
    /// 按会话键哈希选择上行 worker：同一会话的消息始终进入同一队列，
    /// 由该 worker 串行处理以保持会话内顺序。
    /// 队列已满时等待（背压），把压力传导到客户端的发送节奏。
    pub async fn broadcast(&self, msg: Msg) -> Result<(), Error> {
        let key = Self::route_key(&msg);
        let worker_count = self.upstream.len();
        if worker_count == 0 {
            return Err(Error::BroadCastError("上行worker未运行".to_string()));
        }
        let idx = Self::worker_index(&key, worker_count);
        self.upstream[idx]
            .send(msg)
            .await
            .map_err(|e| Error::BroadCastError(e.to_string()))
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

    fn msg(send_id: &str, receiver_id: &str, group_id: &str) -> Msg {
        Msg {
            send_id: send_id.into(),
            receiver_id: receiver_id.into(),
            group_id: group_id.into(),
            msg_type: if group_id.is_empty() {
                MsgType::SingleMsg as i32
            } else {
                MsgType::GroupMsg as i32
            },
            ..Default::default()
        }
    }

    #[test]
    fn route_key_groups_by_conversation() {
        // 群聊：按群ID路由，与发送者无关
        assert_eq!(Manager::route_key(&msg("u1", "", "g1")), "g:g1");
        assert_eq!(Manager::route_key(&msg("u2", "", "g1")), "g:g1");

        // 单聊：A→B 与 B→A 是同一会话，路由键必须一致
        let ab = Manager::route_key(&msg("u1", "u2", ""));
        let ba = Manager::route_key(&msg("u2", "u1", ""));
        assert_eq!(ab, ba);
        assert_eq!(ab, "d:u1:u2");
    }

    #[test]
    fn route_key_is_the_shared_conversation_key() {
        // 跨 crate 契约：网关上行保序键、Kafka 分区键、落库会话序号
        // 必须来自同一推导，防止三处实现各自漂移
        use contract::types::conversation::conversation_id_of;
        for sample in [
            msg("u1", "u2", ""),
            msg("u2", "u1", ""),
            msg("u1", "", "g1"),
            msg("u2", "", "g1"),
            msg("u1", "", ""),
        ] {
            assert_eq!(Manager::route_key(&sample), conversation_id_of(&sample));
        }
    }

    #[test]
    fn worker_index_is_stable_and_in_range() {
        let key = Manager::route_key(&msg("u1", "u2", ""));
        let first = Manager::worker_index(&key, 8);
        // 同一会话永远落在同一 worker（保序的前提）
        assert_eq!(Manager::worker_index(&key, 8), first);
        assert!(first < 8);

        // 边界：1 个 worker 时全部落在 0
        assert_eq!(Manager::worker_index("anything", 1), 0);
    }

    #[test]
    fn route_key_falls_back_to_sender() {
        // 心跳/回执等无对端消息：单聊键退化为按发送者定向，仍是确定性的
        assert_eq!(Manager::route_key(&msg("u1", "", "")), "d::u1");
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
