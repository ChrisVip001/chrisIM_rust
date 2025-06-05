use anyhow::Result;
use tonic::Request;
use prost_types;

use crate::proto::group::group_service_client::GroupServiceClient;
use crate::proto::group::{
    AddMemberRequest, CheckMembershipRequest, CheckMembershipResponse, CreateGroupRequest,
    DeleteGroupRequest, DeleteGroupResponse, GetGroupRequest, GetMembersRequest, GetMembersResponse,
    GetUserGroupsRequest, GetUserGroupsResponse, GroupResponse, MemberResponse, MemberRole,
    RemoveMemberRequest, RemoveMemberResponse, UpdateGroupRequest, UpdateMemberRoleRequest,
    SearchUserGroupsRequest, SearchUserGroupsResponse,
    CreateAnnouncementRequest, AnnouncementResponse, GetAnnouncementRequest, 
    GetGroupAnnouncementsRequest, GetGroupAnnouncementsResponse, DeleteAnnouncementRequest,
    DeleteAnnouncementResponse, GetGroupSettingsRequest, GroupSettingsResponse,
    UpdateGroupSettingsRequest, AddToBlacklistRequest, BlacklistResponse,
    RemoveFromBlacklistRequest, RemoveFromBlacklistResponse, GetBlacklistRequest,
    GetBlacklistResponse, MuteMemberRequest, MuteResponse, UnmuteMemberRequest,
    UnmuteResponse, GetMutedMembersRequest, GetMutedMembersResponse,
    GetMemberSettingsRequest, MemberSettingsResponse, UpdateMemberSettingsRequest,
    CreateGroupQrcodeRequest, GroupQrcodeResponse, GetGroupQrcodeRequest,
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
        members: Vec<String>,
    ) -> Result<GroupResponse> {
        let request = Request::new(CreateGroupRequest {
            name: name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            avatar_url: avatar_url.to_string(),
            members,
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
            page: 1,
            page_size: 20,
        });

        let response = self.service_client.get_members(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组成员列表（带分页）
    pub async fn get_members_with_params(
        &mut self,
        group_id: &str,
        page: i32,
        page_size: i32,
    ) -> Result<GetMembersResponse> {
        let request = Request::new(GetMembersRequest {
            group_id: group_id.to_string(),
            page,
            page_size,
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
    
    /// 搜索用户加入的群组 (按关键字)
    pub async fn search_user_groups(
        &mut self,
        user_id: &str,
        keyword: &str,
        page: i32,
        page_size: i32,
    ) -> Result<SearchUserGroupsResponse> {
        let request = Request::new(SearchUserGroupsRequest {
            user_id: user_id.to_string(),
            keyword: keyword.to_string(),
            page,
            page_size,
        });

        let response = self.service_client.search_user_groups(request).await?;
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

    /// 创建群公告
    pub async fn create_announcement(
        &mut self,
        group_id: &str,
        creator_id: &str,
        title: &str,
        content: &str,
        is_pinned: bool,
    ) -> Result<AnnouncementResponse> {
        let request = Request::new(CreateAnnouncementRequest {
            group_id: group_id.to_string(),
            creator_id: creator_id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            is_pinned,
        });

        let response = self.service_client.create_announcement(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群公告
    pub async fn get_announcement(&mut self, announcement_id: &str) -> Result<AnnouncementResponse> {
        let request = Request::new(GetAnnouncementRequest {
            announcement_id: announcement_id.to_string(),
        });

        let response = self.service_client.get_announcement(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组所有公告
    pub async fn get_group_announcements(&mut self, group_id: &str) -> Result<GetGroupAnnouncementsResponse> {
        let request = Request::new(GetGroupAnnouncementsRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_group_announcements(request).await?;
        Ok(response.into_inner())
    }

    /// 删除群公告
    pub async fn delete_announcement(
        &mut self,
        announcement_id: &str,
        deleted_by_id: &str,
    ) -> Result<DeleteAnnouncementResponse> {
        let request = Request::new(DeleteAnnouncementRequest {
            announcement_id: announcement_id.to_string(),
            deleted_by_id: deleted_by_id.to_string(),
        });

        let response = self.service_client.delete_announcement(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组设置
    pub async fn get_group_settings(&mut self, group_id: &str) -> Result<GroupSettingsResponse> {
        let request = Request::new(GetGroupSettingsRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_group_settings(request).await?;
        Ok(response.into_inner())
    }

    /// 更新群组设置
    pub async fn update_group_settings(
        &mut self,
        group_id: &str,
        updated_by_id: &str,
        allow_member_friendship: bool,
        join_approval_required: bool,
        only_admin_can_invite: bool,
        only_admin_can_modify: bool,
    ) -> Result<GroupSettingsResponse> {
        let request = Request::new(UpdateGroupSettingsRequest {
            group_id: group_id.to_string(),
            updated_by_id: updated_by_id.to_string(),
            allow_member_friendship,
            join_approval_required,
            only_admin_can_invite,
            only_admin_can_modify,
        });

        let response = self.service_client.update_group_settings(request).await?;
        Ok(response.into_inner())
    }

    /// 添加用户到黑名单
    pub async fn add_to_blacklist(
        &mut self,
        group_id: &str,
        user_id: &str,
        creator_id: &str,
        reason: &str,
    ) -> Result<BlacklistResponse> {
        let request = Request::new(AddToBlacklistRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            creator_id: creator_id.to_string(),
            reason: reason.to_string(),
        });

        let response = self.service_client.add_to_blacklist(request).await?;
        Ok(response.into_inner())
    }

    /// 从黑名单中移除用户
    pub async fn remove_from_blacklist(
        &mut self,
        group_id: &str,
        user_id: &str,
        removed_by_id: &str,
    ) -> Result<RemoveFromBlacklistResponse> {
        let request = Request::new(RemoveFromBlacklistRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            removed_by_id: removed_by_id.to_string(),
        });

        let response = self.service_client.remove_from_blacklist(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群组黑名单
    pub async fn get_blacklist(&mut self, group_id: &str) -> Result<GetBlacklistResponse> {
        let request = Request::new(GetBlacklistRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_blacklist(request).await?;
        Ok(response.into_inner())
    }

    /// 禁言成员
    pub async fn mute_member(
        &mut self,
        group_id: &str,
        user_id: &str,
        creator_id: &str,
        reason: &str,
        mute_until: Option<prost_types::Timestamp>,
        is_permanent: bool,
    ) -> Result<MuteResponse> {
        let request = Request::new(MuteMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            creator_id: creator_id.to_string(),
            reason: reason.to_string(),
            mute_until,
            is_permanent,
        });

        let response = self.service_client.mute_member(request).await?;
        Ok(response.into_inner())
    }

    /// 解除成员禁言
    pub async fn unmute_member(
        &mut self,
        group_id: &str,
        user_id: &str,
        unmuted_by_id: &str,
    ) -> Result<UnmuteResponse> {
        let request = Request::new(UnmuteMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            unmuted_by_id: unmuted_by_id.to_string(),
        });

        let response = self.service_client.unmute_member(request).await?;
        Ok(response.into_inner())
    }

    /// 获取禁言成员列表
    pub async fn get_muted_members(&mut self, group_id: &str) -> Result<GetMutedMembersResponse> {
        let request = Request::new(GetMutedMembersRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_muted_members(request).await?;
        Ok(response.into_inner())
    }

    /// 获取成员设置
    pub async fn get_member_settings(
        &mut self,
        group_id: &str,
        user_id: &str,
    ) -> Result<MemberSettingsResponse> {
        let request = Request::new(GetMemberSettingsRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_member_settings(request).await?;
        Ok(response.into_inner())
    }

    /// 更新成员设置
    pub async fn update_member_settings(
        &mut self,
        group_id: &str,
        user_id: &str,
        mute_notifications: bool,
        nickname_in_group: &str,
    ) -> Result<MemberSettingsResponse> {
        let request = Request::new(UpdateMemberSettingsRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
            mute_notifications,
            nickname_in_group: nickname_in_group.to_string(),
        });

        let response = self.service_client.update_member_settings(request).await?;
        Ok(response.into_inner())
    }

    /// 创建群二维码
    pub async fn create_group_qrcode(
        &mut self,
        group_id: &str,
        creator_id: &str,
        expires_at: Option<prost_types::Timestamp>,
        is_permanent: bool,
    ) -> Result<GroupQrcodeResponse> {
        let request = Request::new(CreateGroupQrcodeRequest {
            group_id: group_id.to_string(),
            creator_id: creator_id.to_string(),
            expires_at,
            is_permanent,
        });

        let response = self.service_client.create_group_qrcode(request).await?;
        Ok(response.into_inner())
    }

    /// 获取群二维码
    pub async fn get_group_qrcode(&mut self, group_id: &str) -> Result<GroupQrcodeResponse> {
        let request = Request::new(GetGroupQrcodeRequest {
            group_id: group_id.to_string(),
        });

        let response = self.service_client.get_group_qrcode(request).await?;
        Ok(response.into_inner())
    }
} 