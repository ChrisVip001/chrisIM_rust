use anyhow::Result;
use tonic::Request;

use crate::proto::friend::friend_service_client::FriendServiceClient;
use crate::proto::friend::{
    AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse, DeleteFriendRequest,
    DeleteFriendResponse, FriendshipResponse, GetFriendListRequest, GetFriendListResponse,
    GetFriendRequestsRequest, GetFriendRequestsResponse, RejectFriendRequestRequest,
    SendFriendRequestRequest,
};

use crate::grpc_client::GrpcServiceClient;

/// 好友服务gRPC客户端
#[derive(Clone)]
pub struct FriendServiceGrpcClient {
    service_client: GrpcServiceClient,
}

impl FriendServiceGrpcClient {
    /// 创建新的好友服务客户端
    pub fn new(service_client: GrpcServiceClient) -> Self {
        Self { service_client }
    }

    /// 从环境变量创建客户端
    pub fn from_env() -> Self {
        let service_client = GrpcServiceClient::from_env("friend-service");
        Self::new(service_client)
    }

    /// 发送好友请求
    pub async fn send_friend_request(
        &self,
        user_id: &str,
        friend_id: &str,
        message: &str,
    ) -> Result<FriendshipResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(SendFriendRequestRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
            message: message.to_string(),
        });

        let response = client.send_friend_request(request).await?;
        Ok(response.into_inner())
    }

    /// 接受好友请求
    pub async fn accept_friend_request(
        &self,
        user_id: &str,
        friend_id: &str,
    ) -> Result<FriendshipResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(AcceptFriendRequestRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        });

        let response = client.accept_friend_request(request).await?;
        Ok(response.into_inner())
    }

    /// 拒绝好友请求
    pub async fn reject_friend_request(
        &self,
        user_id: &str,
        friend_id: &str,
        reason: &str,
    ) -> Result<FriendshipResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(RejectFriendRequestRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
            reason: reason.to_string(),
        });

        let response = client.reject_friend_request(request).await?;
        Ok(response.into_inner())
    }

    /// 获取好友列表
    pub async fn get_friend_list(&self, user_id: &str) -> Result<GetFriendListResponse> {
        self.get_friend_list_with_params(user_id, 0, 0, "").await
    }

    /// 获取好友列表（带参数）
    pub async fn get_friend_list_with_params(
        &self,
        user_id: &str,
        page: i64,
        page_size: i64,
        sort_by: &str,
    ) -> Result<GetFriendListResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(GetFriendListRequest {
            user_id: user_id.to_string(),
            page,
            page_size,
            sort_by: sort_by.to_string(),
        });

        let response = client.get_friend_list(request).await?;
        Ok(response.into_inner())
    }

    /// 获取好友请求列表
    pub async fn get_friend_requests(&self, user_id: &str) -> Result<GetFriendRequestsResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(GetFriendRequestsRequest {
            user_id: user_id.to_string(),
        });

        let response = client.get_friend_requests(request).await?;
        Ok(response.into_inner())
    }

    /// 删除好友
    pub async fn delete_friend(&self, user_id: &str, friend_id: &str) -> Result<DeleteFriendResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(DeleteFriendRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        });

        let response = client.delete_friend(request).await?;
        Ok(response.into_inner())
    }

    /// 检查好友关系
    pub async fn check_friendship(
        &self,
        user_id: &str,
        friend_id: &str,
    ) -> Result<CheckFriendshipResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = FriendServiceClient::new(channel);

        let request = Request::new(CheckFriendshipRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        });

        let response = client.check_friendship(request).await?;
        Ok(response.into_inner())
    }
} 