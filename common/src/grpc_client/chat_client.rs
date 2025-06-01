use anyhow::Result;
use tonic::Request;

use crate::proto::message::chat_service_client::ChatServiceClient;
use crate::proto::message::{SendMsgRequest, MsgResponse};
use crate::service_discovery::LbWithServiceDiscovery;

/// 聊天服务gRPC客户端
#[derive(Clone)]
pub struct ChatServiceGrpcClient {
    service_client: ChatServiceClient<LbWithServiceDiscovery>,
}

impl ChatServiceGrpcClient {
    /// 创建新的聊天服务客户端
    pub fn new(service_client: ChatServiceClient<LbWithServiceDiscovery>) -> Self {
        Self { service_client }
    }

    /// 发送消息
    pub async fn send_msg(&mut self, request: SendMsgRequest) -> Result<MsgResponse> {
        let response = self.service_client.send_msg(Request::new(request)).await?;
        Ok(response.into_inner())
    }
} 