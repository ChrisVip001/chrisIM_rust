use anyhow::Result;
use tonic::Request;

use crate::proto::friend::friend_service_client::FriendServiceClient;
use crate::proto::friend::{
    AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse, DeleteFriendRequest,
    DeleteFriendResponse, FriendshipResponse, GetFriendListRequest, GetFriendListResponse,
    GetFriendRequestsRequest, GetFriendRequestsResponse, RejectFriendRequestRequest,
    SendFriendRequestRequest, BlockUserRequest, BlockUserResponse, UnblockUserRequest, UnblockUserResponse,
    CreateOrUpdateFriendGroupRequest, FriendGroupResponse, DeleteFriendGroupRequest,
    DeleteFriendGroupResponse, GetFriendGroupsRequest, GetFriendGroupsResponse,
    GetGroupFriendsRequest, GetGroupFriendsResponse, SearchPotentialFriendsRequest, SearchPotentialFriendsResponse,
    GetAllFriendDetailListRequest, GetAllFriendDetailListResponse,
};

use crate::service_discovery::LbWithServiceDiscovery;

/// 好友服务gRPC客户端
#[derive(Clone)]
pub struct FriendServiceGrpcClient {
    service_client: FriendServiceClient<LbWithServiceDiscovery>,
}

impl FriendServiceGrpcClient {
    /// 创建新的好友服务客户端
    pub fn new(service_client: FriendServiceClient<LbWithServiceDiscovery>) -> Self {
        Self { service_client }
    }

    /// 发送好友请求
    pub async fn send_friend_request(
        &mut self,
        user_id: &str,
        friend_id: &str,
        message: &str,
    ) -> Result<FriendshipResponse> {
        let request = Request::new(SendFriendRequestRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
            message: message.to_string(),
        });

        let response = self.service_client.send_friend_request(request).await?;
        Ok(response.into_inner())
    }

    /// 接受好友请求
    pub async fn accept_friend_request(
        &mut self,
        user_id: &str,
        request_id: &str,
    ) -> Result<FriendshipResponse> {
        let request = Request::new(AcceptFriendRequestRequest {
            user_id: user_id.to_string(),
            request_id: request_id.to_string(),
        });

        let response = self.service_client.accept_friend_request(request).await?;
        Ok(response.into_inner())
    }

    /// 拒绝好友请求
    pub async fn reject_friend_request(
        &mut self,
        user_id: &str,
        reason: &str,
        request_id: &str,
    ) -> Result<FriendshipResponse> {
        let request = Request::new(RejectFriendRequestRequest {
            user_id: user_id.to_string(),
            reason: reason.to_string(),
            request_id: request_id.to_string(),
        });

        let response = self.service_client.reject_friend_request(request).await?;
        Ok(response.into_inner())
    }

    /// 获取好友列表（带参数）
    pub async fn get_friend_list_with_params(
        &mut self,
        user_id: &str,
        page: i64,
        page_size: i64,
        sort_by: &str,
        keyword: &str,
    ) -> Result<GetFriendListResponse> {
        let request = GetFriendListRequest {
            user_id: user_id.to_string(),
            page,
            page_size,
            sort_by: sort_by.to_string(),
            keyword: keyword.to_string(),
        };

        let response = self.service_client.get_friend_list(request).await?;
        Ok(response.into_inner())
    }

    /// 获取好友请求列表（带分页参数）
    pub async fn get_friend_requests_with_params(&mut self, user_id: &str, page: i64, page_size: i64) -> Result<GetFriendRequestsResponse> {
        let request = Request::new(GetFriendRequestsRequest {
            user_id: user_id.to_string(),
            page,
            page_size,
        });

        let response = self.service_client.get_friend_requests(request).await?;
        Ok(response.into_inner())
    }

    /// 删除好友
    pub async fn delete_friend(&mut self, user_id: &str, friend_id: &str) -> Result<DeleteFriendResponse> {
        let request = Request::new(DeleteFriendRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        });

        let response = self.service_client.delete_friend(request).await?;
        Ok(response.into_inner())
    }

    /// 检查好友关系
    pub async fn check_friendship(
        &mut self,
        user_id: &str,
        friend_id: &str,
    ) -> Result<CheckFriendshipResponse> {
        let request = Request::new(CheckFriendshipRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        });

        let response = self.service_client.check_friendship(request).await?;
        Ok(response.into_inner())
    }

    /// 拉黑用户
    pub async fn block_user(&mut self, user_id: &str, blocked_user_id: &str, reason: Option<&str>) -> Result<BlockUserResponse> {
        let request = Request::new(BlockUserRequest {
            user_id: user_id.to_string(),
            blocked_user_id: blocked_user_id.to_string(),
            reason: reason.unwrap_or("").to_string(),
        });

        let response = self.service_client.block_user(request).await?;
        Ok(response.into_inner())
    }

    /// 解除拉黑
    pub async fn unblock_user(&mut self, user_id: &str, blocked_user_id: &str) -> Result<UnblockUserResponse> {
        let request = Request::new(UnblockUserRequest {
            user_id: user_id.to_string(),
            blocked_user_id: blocked_user_id.to_string(),
        });

        let response = self.service_client.unblock_user(request).await?;
        Ok(response.into_inner())
    }

    /// 创建或更新好友分组
    pub async fn create_or_update_friend_group(
        &mut self,
        id: Option<String>,
        user_id: &str,
        group_name: &str,
        sort_order: i32,
        friend_ids: Vec<String>,
    ) -> Result<FriendGroupResponse> {
        let request = Request::new(CreateOrUpdateFriendGroupRequest {
            id: id.map(String::from),
            user_id: user_id.to_string(),
            group_name: group_name.to_string(),
            sort_order,
            friend_ids,
        });

        let response = self.service_client.create_or_update_friend_group(request).await?;
        Ok(response.into_inner())
    }

    /// 删除好友分组
    pub async fn delete_friend_group(&mut self, id: &str, user_id: &str) -> Result<DeleteFriendGroupResponse> {
        let request = Request::new(DeleteFriendGroupRequest {
            id: id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = self.service_client.delete_friend_group(request).await?;
        Ok(response.into_inner())
    }

    /// 获取好友分组列表
    pub async fn get_friend_groups(&mut self, user_id: &str) -> Result<GetFriendGroupsResponse> {

        let request = Request::new(GetFriendGroupsRequest {
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_friend_groups(request).await?;
        Ok(response.into_inner())
    }

    /// 获取分组好友列表
    pub async fn get_group_friends(&mut self, group_id: &str, user_id: &str) -> Result<GetGroupFriendsResponse> {
        let request = Request::new(GetGroupFriendsRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_group_friends(request).await?;
        Ok(response.into_inner())
    }

    /// 搜索潜在好友
    pub async fn search_potential_friends(
        &mut self,
        user_id: &str,
        search_term: &str,
    ) -> Result<SearchPotentialFriendsResponse> {
        let request = Request::new(SearchPotentialFriendsRequest {
            user_id: user_id.to_string(),
            search_term: search_term.to_string(),
        });

        let response = self.service_client.search_potential_friends(request).await?;
        Ok(response.into_inner())
    }

    /// 获取所有好友详细列表（无分页）
    pub async fn get_all_friend_detail_list(
        &mut self,
        user_id: &str,
    ) -> Result<GetAllFriendDetailListResponse> {
        let request = Request::new(GetAllFriendDetailListRequest {
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_all_friend_detail_list(request).await?;
        Ok(response.into_inner())
    }

    /// 设置好友星标状态
    pub async fn toggle_friend_star(
        &mut self,
        user_id: &str,
        friend_id: &str,
        is_starred: bool,
    ) -> Result<crate::proto::friend::ToggleFriendStarResponse> {
        let request = Request::new(crate::proto::friend::ToggleFriendStarRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
            is_starred,
        });
        
        let response = self.service_client.toggle_friend_star(request).await?;
        Ok(response.into_inner())
    }

    /// 设置好友置顶状态
    pub async fn toggle_friend_top(
        &mut self,
        user_id: &str,
        friend_id: &str,
        is_top: bool,
    ) -> Result<crate::proto::friend::ToggleFriendTopResponse> {
        let request = Request::new(crate::proto::friend::ToggleFriendTopRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
            is_top,
        });
        
        let response = self.service_client.toggle_friend_top(request).await?;
        Ok(response.into_inner())
    }

    /// 更新好友备注
    pub async fn update_friend_remark(
        &mut self,
        user_id: &str,
        friend_id: &str,
        remark: &str,
    ) -> Result<crate::proto::friend::UpdateFriendRemarkResponse> {
        let request = Request::new(crate::proto::friend::UpdateFriendRemarkRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
            remark: remark.to_string(),
        });

        let response = self.service_client.update_friend_remark(request).await?;
        Ok(response.into_inner())
    }

    /// 获取用户黑名单列表
    pub async fn get_user_blacklist(
        &mut self,
        user_id: &str,
    ) -> Result<crate::proto::friend::GetUserBlacklistResponse> {
        let request = Request::new(crate::proto::friend::GetUserBlacklistRequest {
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_user_blacklist(request).await?;
        Ok(response.into_inner())
    }
}