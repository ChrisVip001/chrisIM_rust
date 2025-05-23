use anyhow::Result;
use tonic::Request;

use crate::proto::group::group_service_client::GroupServiceClient;
use crate::proto::group::{
    AddMemberRequest, CheckMembershipRequest, CheckMembershipResponse, CreateGroupRequest,
    DeleteGroupRequest, DeleteGroupResponse, GetGroupRequest, GetMembersRequest, GetMembersResponse,
    GetUserGroupsRequest, GetUserGroupsResponse, GroupResponse, MemberResponse, MemberRole,
    RemoveMemberRequest, RemoveMemberResponse, UpdateGroupRequest, UpdateMemberRoleRequest,
};

use crate::service_discovery::LbWithServiceDiscovery;

/// 群组服务gRPC客户端
#[derive(Clone)]
pub struct GroupServiceGrpcClient {
    service_client: GroupServiceClient<LbWithServiceDiscovery>,
}

impl GroupServiceGrpcClient {
    /// 创建新的群组服务客户端
    pub fn new(service_client: GroupServiceClient<LbWithServiceDiscovery>) -> Self {
        Self { service_client }
    }

    /// 创建群组
    pub async fn create_group(
        &mut self,
        name: &str,
        description: &str,
        owner_id: &str,
        avatar_url: &str,
    ) -> Result<GroupResponse> {
        let request = Request::new(CreateGroupRequest {
            name: name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            avatar_url: avatar_url.to_string(),
        });

        let response = self.service_client.create_group(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组信息
    pub async fn get_group(&mut self, group_id: &str) -> Result<GroupResponse> {
        let request = Request::new(GetGroupRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_group(request).await?;
        Ok(response.into_inner())
    }

    /// 更新群组信息
    pub async fn update_group(
        &mut self,
        group_id: &str,
        name: Option<String>,
        description: Option<String>,
        avatar_url: Option<String>,
    ) -> Result<GroupResponse> {
        let request = Request::new(UpdateGroupRequest {
            group_id: group_id.to_string(),
            name,
            description,
            avatar_url,
        });

        let response = self.service_client.update_group(request).await?;
        Ok(response.into_inner())
    }

    /// 删除群组
    pub async fn delete_group(&mut self, group_id: &str, user_id: &str) -> Result<DeleteGroupResponse> {
        let request = Request::new(DeleteGroupRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = self.service_client.delete_group(request).await?;
        Ok(response.into_inner())
    }

    /// 添加群组成员
    pub async fn add_member(
        &mut self,
        group_id: &str,
        user_id: &str,
        added_by_id: &str,
        role: MemberRole,
    ) -> Result<MemberResponse> {

        let request = Request::new(AddMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            added_by_id: added_by_id.to_string(),
            role: role as i32,
        });

        let response = self.service_client.add_member(request).await?;
        Ok(response.into_inner())
    }

    /// 移除群组成员
    pub async fn remove_member(
        &mut self,
        group_id: &str,
        user_id: &str,
        removed_by_id: &str,
    ) -> Result<RemoveMemberResponse> {
        let request = Request::new(RemoveMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            removed_by_id: removed_by_id.to_string(),
        });

        let response = self.service_client.remove_member(request).await?;
        Ok(response.into_inner())
    }

    /// 更新成员角色
    pub async fn update_member_role(
        &mut self,
        group_id: &str,
        user_id: &str,
        updated_by_id: &str,
        role: MemberRole,
    ) -> Result<MemberResponse> {
        let request = Request::new(UpdateMemberRoleRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            updated_by_id: updated_by_id.to_string(),
            role: role as i32,
        });

        let response = self.service_client.update_member_role(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组成员列表
    pub async fn get_members(&mut self, group_id: &str) -> Result<GetMembersResponse> {
        let request = Request::new(GetMembersRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_members(request).await?;
        Ok(response.into_inner())
    }

    /// 获取用户加入的群组列表
    pub async fn get_user_groups(&mut self, user_id: &str) -> Result<GetUserGroupsResponse> {
        let request = Request::new(GetUserGroupsRequest {
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_user_groups(request).await?;
        Ok(response.into_inner())
    }

    /// 检查用户是否在群组中
    pub async fn check_membership(
        &mut self,
        group_id: &str,
        user_id: &str,
    ) -> Result<CheckMembershipResponse> {

        let request = Request::new(CheckMembershipRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = self.service_client.check_membership(request).await?;
        Ok(response.into_inner())
    }
} 