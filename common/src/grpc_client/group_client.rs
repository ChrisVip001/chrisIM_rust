use anyhow::Result;
use tonic::Request;

use crate::proto::group::group_service_client::GroupServiceClient;
use crate::proto::group::{
    AddMemberRequest, CheckMembershipRequest, CheckMembershipResponse, CreateGroupRequest,
    DeleteGroupRequest, DeleteGroupResponse, GetGroupRequest, GetMembersRequest, GetMembersResponse,
    GetUserGroupsRequest, GetUserGroupsResponse, GroupResponse, MemberResponse, MemberRole,
    RemoveMemberRequest, RemoveMemberResponse, UpdateGroupRequest, UpdateMemberRoleRequest,
};

use crate::grpc_client::GrpcServiceClient;

/// 群组服务gRPC客户端
#[derive(Clone)]
pub struct GroupServiceGrpcClient {
    service_client: GrpcServiceClient,
}

impl GroupServiceGrpcClient {
    /// 创建新的群组服务客户端
    pub fn new(service_client: GrpcServiceClient) -> Self {
        Self { service_client }
    }

    /// 从环境变量创建客户端
    pub fn from_env() -> Self {
        let service_client = GrpcServiceClient::from_env("group-service");
        Self::new(service_client)
    }

    /// 创建群组
    pub async fn create_group(
        &self,
        name: &str,
        description: &str,
        owner_id: &str,
        avatar_url: &str,
    ) -> Result<GroupResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(CreateGroupRequest {
            name: name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            avatar_url: avatar_url.to_string(),
        });

        let response = client.create_group(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组信息
    pub async fn get_group(&self, group_id: &str) -> Result<GroupResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(GetGroupRequest {
            group_id: group_id.to_string(),
        });

        let response = client.get_group(request).await?;
        Ok(response.into_inner())
    }

    /// 更新群组信息
    pub async fn update_group(
        &self,
        group_id: &str,
        name: Option<String>,
        description: Option<String>,
        avatar_url: Option<String>,
    ) -> Result<GroupResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(UpdateGroupRequest {
            group_id: group_id.to_string(),
            name,
            description,
            avatar_url,
        });

        let response = client.update_group(request).await?;
        Ok(response.into_inner())
    }

    /// 删除群组
    pub async fn delete_group(&self, group_id: &str, user_id: &str) -> Result<DeleteGroupResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(DeleteGroupRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = client.delete_group(request).await?;
        Ok(response.into_inner())
    }

    /// 添加群组成员
    pub async fn add_member(
        &self,
        group_id: &str,
        user_id: &str,
        added_by_id: &str,
        role: MemberRole,
    ) -> Result<MemberResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(AddMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            added_by_id: added_by_id.to_string(),
            role: role as i32,
        });

        let response = client.add_member(request).await?;
        Ok(response.into_inner())
    }

    /// 移除群组成员
    pub async fn remove_member(
        &self,
        group_id: &str,
        user_id: &str,
        removed_by_id: &str,
    ) -> Result<RemoveMemberResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(RemoveMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            removed_by_id: removed_by_id.to_string(),
        });

        let response = client.remove_member(request).await?;
        Ok(response.into_inner())
    }

    /// 更新成员角色
    pub async fn update_member_role(
        &self,
        group_id: &str,
        user_id: &str,
        updated_by_id: &str,
        role: MemberRole,
    ) -> Result<MemberResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(UpdateMemberRoleRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            updated_by_id: updated_by_id.to_string(),
            role: role as i32,
        });

        let response = client.update_member_role(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组成员列表
    pub async fn get_members(&self, group_id: &str) -> Result<GetMembersResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(GetMembersRequest {
            group_id: group_id.to_string(),
        });

        let response = client.get_members(request).await?;
        Ok(response.into_inner())
    }

    /// 获取用户加入的群组列表
    pub async fn get_user_groups(&self, user_id: &str) -> Result<GetUserGroupsResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(GetUserGroupsRequest {
            user_id: user_id.to_string(),
        });

        let response = client.get_user_groups(request).await?;
        Ok(response.into_inner())
    }

    /// 检查用户是否在群组中
    pub async fn check_membership(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<CheckMembershipResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = GroupServiceClient::new(channel);

        let request = Request::new(CheckMembershipRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = client.check_membership(request).await?;
        Ok(response.into_inner())
    }
} 