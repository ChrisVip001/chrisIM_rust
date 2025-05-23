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
/// 负责与多个WebSocket网关通信，将消息推送给在线客户端
#[derive(Debug)]
pub struct PusherService {
    // 带负载均衡和服务发现的WebSocket RPC客户端
    ws_rpc_client: MsgServiceClient<LbWithServiceDiscovery>,
}

impl PusherService {
    /// 创建一个新的推送服务实例
    /// 使用项目的服务发现机制初始化WebSocket连接
    pub async fn new(config: &AppConfig) -> Result<Self, Error> {
        // 获取WebSocket网关服务名称
        let sub_svr_name = config.rpc.ws.name.clone();

        // 使用项目的服务发现机制创建带负载均衡的通道
        let channel = get_chan(config, sub_svr_name).await?;
        
        // 创建WebSocket RPC客户端
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
    /// 将消息发送到WebSocket网关，由网关转发给目标用户
    async fn push_single_msg(&self, request: Msg) -> Result<(), Error> {
        debug!("推送单聊消息请求: {:?}", request);

        // 构建发送消息请求
        let request = SendMsgRequest {
            message: Some(request),
        };

        // 使用带负载均衡的客户端发送消息
        let mut client = self.ws_rpc_client.clone();
        match client.send_msg_to_user(request).await {
            Ok(_) => {
                debug!("单聊消息推送成功");
                Ok(())
            }
            Err(err) => {
                error!("推送单聊消息失败: {}", err);
                Err(Error::Internal(format!("推送单聊消息失败: {}", err)))
            }
        }
    }

    /// 推送群聊消息
    /// 将消息发送到WebSocket网关，由网关转发给群成员
    async fn push_group_msg(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error> {
        debug!("推送群聊消息请求: {:?}, 成员: {:?}", msg, members);

        // 构建群聊消息请求
        let request = SendGroupMsgRequest {
            message: Some(msg),
            members,
        };

        // 使用带负载均衡的客户端发送群聊消息
        let mut client = self.ws_rpc_client.clone();
        match client.send_group_msg_to_user(request).await {
            Ok(_) => {
                debug!("群聊消息推送成功");
                Ok(())
            }
            Err(err) => {
                error!("推送群聊消息失败: {}", err);
                Err(Error::Internal(format!("推送群聊消息失败: {}", err)))
            }
        }
    }
}
