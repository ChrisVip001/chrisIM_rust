use async_trait::async_trait;
use common::error::Error;
use tracing::{debug, error, info};

use super::Pusher;
use common::config::AppConfig;
use common::message::msg_service_client::MsgServiceClient;
use common::message::{GroupMemSeq, Msg, SendGroupMsgRequest, SendMsgRequest};
use common::grpc_client::base::get_chan;
use common::service_discovery::LbWithServiceDiscovery;

/// 消息推送服务的具体实现
/// 
/// 负责与多个WebSocket网关通信，将消息推送给在线客户端。
/// 使用gRPC协议与WebSocket网关服务通信，支持服务发现和负载均衡。
/// 
/// ## 工作原理
/// 
/// 1. **服务发现**: 通过Consul自动发现可用的WebSocket网关实例
/// 2. **负载均衡**: 使用轮询算法在多个网关实例间分发请求
/// 3. **消息推送**: 将消息通过gRPC发送给WebSocket网关
/// 4. **错误处理**: 处理网络异常和服务不可用的情况
/// 
/// ## 推送流程
/// 
/// ```text
/// ConsumerService -> PusherService -> WebSocket网关 -> 客户端WebSocket连接
/// ```
#[derive(Debug)]
pub struct PusherService {
    /// 带负载均衡和服务发现的WebSocket RPC客户端
    /// 
    /// 这个客户端会自动：
    /// - 从Consul发现可用的WebSocket网关实例
    /// - 在多个实例间进行负载均衡
    /// - 处理实例故障和自动重连
    /// - 维护连接池以提高性能
    ws_rpc_client: MsgServiceClient<LbWithServiceDiscovery>,
}

impl PusherService {
    /// 创建一个新的推送服务实例
    /// 
    /// 使用项目的服务发现机制初始化WebSocket连接，
    /// 自动配置负载均衡和故障转移功能。
    /// 
    /// # 参数
    /// * `config` - 应用程序配置，包含服务发现和WebSocket服务配置
    /// 
    /// # 返回值
    /// * `Ok(PusherService)` - 成功创建的推送服务实例
    /// * `Err(Error)` - 初始化失败的错误信息
    /// 
    /// # 错误情况
    /// - Consul连接失败
    /// - WebSocket服务未注册
    /// - 网络配置错误
    pub async fn new(config: &AppConfig) -> Result<Self, Error> {
        // 获取WebSocket网关服务名称
        // 这个名称必须与WebSocket网关在Consul中注册的服务名一致
        let sub_svr_name = config.rpc.ws.name.clone();

        // 使用项目的服务发现机制创建带负载均衡的通道
        // 这会自动从Consul查询可用的WebSocket网关实例
        let channel = get_chan(config, sub_svr_name).await?;
        
        // 创建WebSocket RPC客户端
        // 客户端会自动处理连接池、重试、超时等功能
        let ws_rpc_client = MsgServiceClient::new(channel);

        info!("WebSocket服务发现和负载均衡客户端初始化完成");

        Ok(Self {
            ws_rpc_client,
        })
    }
}

#[async_trait]
impl Pusher for PusherService {
    /// 推送单聊消息
    /// 
    /// 将消息发送到WebSocket网关，由网关转发给目标用户。
    /// 如果用户在线，消息会立即通过WebSocket连接推送；
    /// 如果用户离线，消息已经保存在MongoDB消息盒子中，用户上线后会自动拉取。
    /// 
    /// # 参数
    /// * `request` - 要推送的消息对象，包含发送者、接收者、内容等信息
    /// 
    /// # 返回值
    /// * `Ok(())` - 推送成功（包括用户离线的情况）
    /// * `Err(Error)` - 推送失败，通常是网络或服务异常
    /// 
    /// # 推送策略
    /// - 消息会发送给所有在线的WebSocket网关实例
    /// - 网关会检查用户是否在当前实例上在线
    /// - 只有用户在线的网关才会实际推送消息
    /// - 如果所有网关都返回用户离线，这是正常情况
    async fn push_single_msg(&self, request: Msg) -> Result<(), Error> {
        debug!("推送单聊消息请求: server_id={}, 发送者={}, 接收者={}", 
               request.server_id, request.send_id, request.receiver_id);

        // 构建发送消息请求
        let request = SendMsgRequest {
            message: Some(request),
        };

        // 使用带负载均衡的客户端发送消息
        // 客户端会自动选择一个可用的WebSocket网关实例
        let mut client = self.ws_rpc_client.clone();
        match client.send_msg_to_user(request).await {
            Ok(response) => {
                debug!("单聊消息推送成功: {:?}", response);
                Ok(())
            }
            Err(err) => {
                error!("推送单聊消息失败: {}", err);
                Err(Error::Internal(format!("推送单聊消息失败: {}", err)))
            }
        }
    }

    /// 推送群聊消息
    /// 
    /// 将消息发送到WebSocket网关，由网关转发给群成员。
    /// 每个群成员都会收到带有自己序列号的消息副本。
    /// 
    /// # 参数
    /// * `msg` - 要推送的群聊消息
    /// * `members` - 群成员列表，包含每个成员的ID和序列号信息
    /// 
    /// # 返回值
    /// * `Ok(())` - 推送成功
    /// * `Err(Error)` - 推送失败，通常是网络或服务异常
    /// 
    /// # 群聊推送特点
    /// - 消息会发送给所有WebSocket网关实例
    /// - 每个网关检查哪些群成员在当前实例上在线
    /// - 为每个在线成员推送带有其序列号的消息副本
    /// - 离线成员的消息已保存在MongoDB中，上线后自动拉取
    /// - 发送者不会收到自己发送的消息（已在客户端显示）
    async fn push_group_msg(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error> {
        debug!("推送群聊消息请求: server_id={}, 群组={}, 成员数量={}", 
               msg.server_id, msg.group_id, members.len());

        // 构建群聊消息请求
        let request = SendGroupMsgRequest {
            message: Some(msg),
            members,
        };

        // 使用带负载均衡的客户端发送群聊消息
        // 网关会处理群成员的批量推送逻辑
        let mut client = self.ws_rpc_client.clone();
        match client.send_group_msg_to_user(request).await {
            Ok(response) => {
                debug!("群聊消息推送成功: {:?}", response);
                Ok(())
            }
            Err(err) => {
                error!("推送群聊消息失败: {}", err);
                Err(Error::Internal(format!("推送群聊消息失败: {}", err)))
            }
        }
    }
}
