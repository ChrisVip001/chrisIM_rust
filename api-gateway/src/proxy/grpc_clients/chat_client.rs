// 统一错误类型：保留 tonic::Status，由上层按 gRPC 码映射 HTTP 状态
type Result<T> = std::result::Result<T, tonic::Status>;
use tonic::Request;

use contract::proto::message::chat_service_client::ChatServiceClient;
use contract::proto::message::{
    DeleteMessagesRequest, DeleteMessagesResponse, ForwardMessageRequest, ForwardMessageResponse,
    GetConversationsRequest, GetConversationsResponse, GetDbMessagesRequest,
    GetMessageHistoryResponse, MarkConversationAsReadRequest, MarkMessagesAsReadRequest,
    MarkMessagesAsReadResponse, Msg, RevokeMessageRequest, RevokeMessageResponse, SendMsgRequest,
};
use runtime::service_discovery::LbWithServiceDiscovery;

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

    /// 发送消息到 msg-server
    pub async fn send_msg(&mut self, request: SendMsgRequest) -> Result<Msg> {
        let response = self.service_client.send_msg(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 按消息序列号标记已读
    pub async fn mark_messages_as_read(
        &mut self,
        request: MarkMessagesAsReadRequest,
    ) -> Result<MarkMessagesAsReadResponse> {
        let response = self
            .service_client
            .mark_messages_as_read(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }

    /// 按会话标记已读（游标推进到 up_to_time）
    pub async fn mark_conversation_as_read(
        &mut self,
        request: MarkConversationAsReadRequest,
    ) -> Result<MarkMessagesAsReadResponse> {
        let response = self
            .service_client
            .mark_conversation_as_read(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }

    /// 按会话与序列号区间获取消息历史
    pub async fn get_message_history(
        &mut self,
        request: GetDbMessagesRequest,
    ) -> Result<GetMessageHistoryResponse> {
        let response = self
            .service_client
            .get_message_history(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }

    /// 获取会话列表（支持增量同步与按会话上报水位）
    pub async fn get_conversations(
        &mut self,
        request: GetConversationsRequest,
    ) -> Result<GetConversationsResponse> {
        let response = self
            .service_client
            .get_conversations(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }

    /// 撤回消息
    pub async fn revoke_message(
        &mut self,
        request: RevokeMessageRequest,
    ) -> Result<RevokeMessageResponse> {
        let response = self
            .service_client
            .revoke_message(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }

    /// 删除消息
    pub async fn delete_messages(
        &mut self,
        request: DeleteMessagesRequest,
    ) -> Result<DeleteMessagesResponse> {
        let response = self
            .service_client
            .delete_messages(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }

    /// 转发消息
    pub async fn forward_message(
        &mut self,
        request: ForwardMessageRequest,
    ) -> Result<ForwardMessageResponse> {
        let response = self
            .service_client
            .forward_message(Request::new(request))
            .await?;
        Ok(response.into_inner())
    }
}
