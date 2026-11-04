use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use contract::error::Error;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use super::Pusher;
use cache::PresenceCache;
use runtime::config::AppConfig;
use contract::proto::message::msg_service_client::MsgServiceClient;
use contract::proto::message::{GroupMemSeq, Msg, SendGroupMsgRequest, SendMsgRequest};
use runtime::grpc_client::base::{get_chan, service_names as sn};
use runtime::service_discovery::LbWithServiceDiscovery;

/// 定向投递的显式结果
///
/// 过去用 `Err` 兼作「需要广播兜底」的控制信号，调用方必须理解这个错误模式；
/// `directed_single` 甚至因为「成功会把 last_err 置空」而把部分失败当成全部成功，
/// 失败网关上的用户静默丢消息。现在结果自己说明哪些目标没送达，
/// 是否需要兜底由 `needs_broadcast` 一处判定。
#[derive(Debug)]
enum DirectedOutcome {
    /// 所有目标网关都已送达
    Delivered,
    /// 部分或全部目标未送达，记录失败的网关地址
    Partial { failed: Vec<String> },
    /// 无可用路由（用户离线或路由未就绪）
    Unroutable,
}

impl DirectedOutcome {
    fn needs_broadcast(&self) -> bool {
        !matches!(self, Self::Delivered)
    }
}

/// 消息推送服务的具体实现
///
/// 负责与多个WebSocket网关通信，将消息推送给在线客户端。
///
/// ## 推送策略：定向优先，广播兜底
///
/// 1. **定向路由**：网关在连接注册时把「用户平台 → 网关地址」写入 Redis
///    路由表；推送前批量查询路由，把消息直接发到目标网关实例，
    /// 扇出成本从 O(实例数) 降为 O(1)。
/// 2. **广播兜底**：路由查询失败、路由为空（数据未就绪）或定向投递失败
///    （网关崩溃留下残留路由）时，回退为向全部网关实例广播，
///    由各网关本地判断用户是否在线。可用性不回退。
#[derive(Debug)]
pub struct PusherService {
    /// 带负载均衡和服务发现的WebSocket RPC客户端（广播兜底通道）
    ws_rpc_client: MsgServiceClient<LbWithServiceDiscovery>,

    /// 缓存接口（读取网关路由表）
    cache: Arc<dyn PresenceCache>,

    /// 网关直连客户端池：addr → gRPC客户端
    ///
    /// Channel 内部自带连接复用与重连，懒建立。
    direct_clients: DashMap<String, MsgServiceClient<Channel>>,
}

impl PusherService {
    /// 创建一个新的推送服务实例
    pub async fn new(config: &AppConfig) -> Result<Self, Error> {
        // 获取WebSocket网关服务名称
        // 这个名称必须与WebSocket网关在Consul中注册的服务名一致
        let sub_svr_name = sn::WS.to_string();

        // 使用项目的服务发现机制创建带负载均衡的通道
        let channel = get_chan(config, sub_svr_name).await?;
        let ws_rpc_client = MsgServiceClient::new(channel);

        let cache: Arc<dyn PresenceCache> = cache::cache(config).await;

        info!("WebSocket服务发现和负载均衡客户端初始化完成（推送定向路由已启用）");

        Ok(Self {
            ws_rpc_client,
            cache,
            direct_clients: DashMap::new(),
        })
    }

    /// 获取指定网关地址的直连客户端（懒建立，池化复用）
    async fn direct_client(&self, addr: &str) -> Result<MsgServiceClient<Channel>, Error> {
        if let Some(client) = self.direct_clients.get(addr) {
            return Ok(client.clone());
        }
        let endpoint = Channel::from_shared(format!("http://{}", addr))
            .map_err(|e| Error::Internal(format!("网关地址 '{}' 无效: {}", addr, e)))?;
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| Error::Internal(format!("连接网关 '{}' 失败: {}", addr, e)))?;
        let client = MsgServiceClient::new(channel);
        self.direct_clients.insert(addr.to_string(), client.clone());
        debug!("已建立网关 '{}' 直连通道", addr);
        Ok(client)
    }

    /// 广播兜底：单聊推送到全部网关实例
    async fn broadcast_single(&self, msg: Msg) -> Result<(), Error> {
        let mut client = self.ws_rpc_client.clone();
        client
            .send_msg_to_user(SendMsgRequest { message: Some(msg) })
            .await
            .map(|_| ())
            .map_err(|e| Error::Internal(format!("推送单聊消息失败: {}", e)))
    }

    /// 广播兜底：群聊推送到全部网关实例
    async fn broadcast_group(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error> {
        let mut client = self.ws_rpc_client.clone();
        client
            .send_group_msg_to_user(SendGroupMsgRequest {
                message: Some(msg),
                members,
            })
            .await
            .map(|_| ())
            .map_err(|e| Error::Internal(format!("推送群聊消息失败: {}", e)))
    }

    /// 定向推送单聊消息到用户所在网关
    ///
    /// 返回显式结果（见 [`DirectedOutcome`]）；`Err` 只表示路由查询本身失败。
    async fn directed_single(&self, msg: &Msg) -> Result<DirectedOutcome, Error> {
        let routes = self
            .cache
            .get_users_gateway_routes(std::slice::from_ref(&msg.receiver_id))
            .await
            .map_err(|e| Error::Internal(format!("查询网关路由失败: {}", e)))?;

        let addrs = routes.into_iter().flatten().collect::<Vec<_>>();
        if addrs.is_empty() {
            // 路由为空：用户可能确实离线（消息已落库，上线后拉取），
            // 也可能路由尚未写入；交给广播判断，语义不变
            return Ok(DirectedOutcome::Unroutable);
        }

        let mut failed = Vec::new();
        for addr in &addrs {
            let result = match self.direct_client(addr).await {
                Ok(mut client) => client
                    .send_msg_to_user(SendMsgRequest {
                        message: Some(msg.clone()),
                    })
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            if let Err(e) = result {
                warn!("定向推送网关 '{}' 失败: {}，剔除连接并兜底", addr, e);
                self.direct_clients.remove(addr);
                failed.push(addr.clone());
            }
        }

        Ok(if failed.is_empty() {
            DirectedOutcome::Delivered
        } else {
            DirectedOutcome::Partial { failed }
        })
    }

    /// 定向推送群聊消息：按目标网关分组投递
    ///
    /// 返回显式结果（见 [`DirectedOutcome`]）；`Err` 只表示路由查询本身失败。
    async fn directed_group(&self, msg: &Msg, members: &[GroupMemSeq]) -> Result<DirectedOutcome, Error> {
        let member_ids: Vec<String> = members.iter().map(|m| m.mem_id.clone()).collect();
        let routes = self
            .cache
            .get_users_gateway_routes(&member_ids)
            .await
            .map_err(|e| Error::Internal(format!("批量查询网关路由失败: {}", e)))?;

        // 按网关地址归集成员：同一成员多平台可能分布在不同网关
        let mut by_addr: HashMap<String, Vec<GroupMemSeq>> = HashMap::new();
        for (mem, addrs) in members.iter().zip(routes) {
            for addr in addrs {
                by_addr.entry(addr).or_default().push(mem.clone());
            }
        }
        if by_addr.is_empty() {
            return Ok(DirectedOutcome::Unroutable);
        }

        let mut failed = Vec::new();
        for (addr, members) in &by_addr {
            let result = match self.direct_client(addr).await {
                Ok(mut client) => client
                    .send_group_msg_to_user(SendGroupMsgRequest {
                        message: Some(msg.clone()),
                        members: members.clone(),
                    })
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            if let Err(e) = result {
                warn!("定向推送网关 '{}' 失败: {}，剔除连接并兜底", addr, e);
                self.direct_clients.remove(addr);
                failed.push(addr.clone());
            }
        }

        Ok(if failed.is_empty() {
            DirectedOutcome::Delivered
        } else {
            // 部分失败也要兜底：此前 any_success 会让失败网关上的成员静默丢消息，
            // 只依赖上线拉取补偿。推送按「至少一次」语义，由客户端按 server_id 去重。
            DirectedOutcome::Partial { failed }
        })
    }
}

#[async_trait]
impl Pusher for PusherService {
    /// 推送单聊消息
    ///
    /// 优先按路由表定向投递到用户所在网关；路由不可用时广播兜底。
    /// 用户离线时消息已在数据库中，上线后拉取，推送失败不影响最终一致性。
    async fn push_single_msg(&self, request: Msg) -> Result<(), Error> {
        debug!("推送单聊消息请求: server_id={}, 发送者={}, 接收者={}",
               request.server_id, request.send_id, request.receiver_id);

        let outcome = self.directed_single(&request).await.unwrap_or_else(|e| {
            warn!("单聊定向路由查询失败（{}），广播兜底", e);
            DirectedOutcome::Partial { failed: vec!["route-query".into()] }
        });
        if outcome.needs_broadcast() {
            debug!("单聊消息定向推送未完成（{:?}），广播兜底", outcome);
            self.broadcast_single(request).await
        } else {
            debug!("单聊消息定向推送成功: server_id={}", request.server_id);
            Ok(())
        }
    }

    /// 推送群聊消息
    ///
    /// 批量查询成员路由后按网关分组定向投递；只要有一个目标未送达就广播兜底
    /// （至少一次语义，客户端按 server_id 去重）。路由不可用时同样兜底。
    async fn push_group_msg(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error> {
        debug!("推送群聊消息请求: server_id={}, 群组={}, 成员数量={}",
               msg.server_id, msg.group_id, members.len());

        let outcome = self.directed_group(&msg, &members).await.unwrap_or_else(|e| {
            warn!("群聊定向路由查询失败（{}），广播兜底", e);
            DirectedOutcome::Partial { failed: vec!["route-query".into()] }
        });
        if outcome.needs_broadcast() {
            debug!("群聊消息定向推送未完成（{:?}），广播兜底", outcome);
            self.broadcast_group(msg, members).await
        } else {
            debug!("群聊消息定向推送成功: server_id={}", msg.server_id);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DirectedOutcome;

    #[test]
    fn fallback_decision_is_single_sourced() {
        assert!(!DirectedOutcome::Delivered.needs_broadcast());
        assert!(DirectedOutcome::Unroutable.needs_broadcast());
        // 回归：部分失败（先失败后成功）必须兜底，此前单聊会静默丢消息
        assert!(DirectedOutcome::Partial {
            failed: vec!["gateway-a".into()]
        }
        .needs_broadcast());
    }
}
