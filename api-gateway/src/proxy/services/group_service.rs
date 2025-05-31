use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug};
use chrono::{DateTime, TimeZone, Utc};
use std::time::{Duration as StdDuration, SystemTime};
use tonic::transport::Channel;
use common::proto::group::group_service_client::GroupServiceClient;
use prost_types;
use super::common::{
    success_response, extract_string_param, get_optional_string, 
    get_i64_param, timestamp_to_datetime_string, get_user_id_from_jwt,
    get_bool_param,
};
use crate::auth::jwt::UserInfo;

/// 群组服务处理器
#[derive(Clone)]
pub struct GroupServiceHandler;

impl GroupServiceHandler {
    /// 处理群组服务请求
    pub async fn handle_request(
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<UserInfo>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理群组服务请求: {} {}", method, path);

        // 获取群组服务客户端
        let mut client = common::service::group_client().await?;

        // 从JWT中获取用户ID
        let user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;

        // 从路径提取方法名 - 格式: /api/groups/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 创建群组
            (&Method::POST, "create") => {
                let name = extract_string_param(&body, "name", None)?;
                
                let description = body.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                let avatar_url = body.get("avatarUrl")
                    .or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                // 处理初始成员列表
                let mut members = Vec::new();
                if let Some(member_ids) = body.get("members").and_then(|v| v.as_array()) {
                    for member_id in member_ids {
                        if let Some(member_user_id) = member_id.as_str() {
                            members.push(member_user_id.to_string());
                        }
                    }
                }

                let request = proto::group::CreateGroupRequest {
                    name: name.clone(),
                    description,
                    owner_id: user_id.clone(),
                    avatar_url,
                    members,
                };

                let response = client.create_group(request).await?;
                let inner = response.into_inner();

                let group = inner.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(Self::convert_group_to_json(&group), StatusCode::OK))
            }

            // 获取群组信息
            (&Method::GET, "getInfo") | (&Method::GET, "get") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetGroupRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_group(request).await?;
                let inner = response.into_inner();
                let group = inner.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(Self::convert_group_to_json(&group), StatusCode::OK))
            }

            // 更新群组信息
            (&Method::POST, "update") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                
                let name = get_optional_string(&body, "name", None);
                let description = get_optional_string(&body, "description", None);
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url"));

                let request = proto::group::UpdateGroupRequest {
                    group_id: group_id.clone(),
                    name,
                    description,
                    avatar_url,
                };

                let response = client.update_group(request).await?;
                let inner = response.into_inner();
                
                let group = inner.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(Self::convert_group_to_json(&group), StatusCode::OK))
            }

            // 删除群组
            (&Method::GET, "delete") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::DeleteGroupRequest {
                    group_id: group_id.clone(),
                    user_id: user_id.clone(),
                };

                let response = client.delete_group(request).await?;
                let inner = response.into_inner();

                Ok(success_response(
                    inner.success,
                    StatusCode::OK,
                ))
            }

            // 添加成员
            (&Method::POST, "addMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let member_id = extract_string_param(&body, "userId", Some("user_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member as i32,
                    1 => proto::group::MemberRole::Admin as i32,
                    2 => proto::group::MemberRole::Owner as i32,
                    _ => proto::group::MemberRole::Member as i32,
                };

                let request = proto::group::AddMemberRequest {
                    group_id: group_id.clone(),
                    user_id: member_id.clone(),
                    added_by_id: user_id.clone(),
                    role,
                };

                let response = client.add_member(request).await?;
                let inner = response.into_inner();
                let member = inner.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(Self::convert_member_to_json(&member), StatusCode::OK))
            }

            // 移除成员
            (&Method::POST, "removeMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let member_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let request = proto::group::RemoveMemberRequest {
                    group_id: group_id.clone(),
                    user_id: member_id.clone(),
                    removed_by_id: user_id.clone(),
                };

                let response = client.remove_member(request).await?;
                let inner = response.into_inner();
                
                Ok(success_response(
                    inner.success,
                    StatusCode::OK
                ))
            }

            // 更新成员角色
            (&Method::POST, "updateMemberRole") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let member_id = extract_string_param(&body, "userId", Some("user_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member as i32,
                    1 => proto::group::MemberRole::Admin as i32,
                    2 => proto::group::MemberRole::Owner as i32,
                    _ => proto::group::MemberRole::Member as i32,
                };

                let request = proto::group::UpdateMemberRoleRequest {
                    group_id: group_id.clone(),
                    user_id: member_id.clone(),
                    updated_by_id: user_id,
                    role,
                };

                let response = client.update_member_role(request).await?;
                let inner = response.into_inner();
                let member = inner.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(Self::convert_member_to_json(&member), StatusCode::OK))
            }

            // 获取群组成员列表
            (&Method::GET, "getMembers") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetMembersRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_members(request).await?;
                let members = response.into_inner().members.iter().map(|m| Self::convert_member_to_json(m)).collect::<Vec<_>>();

                Ok(success_response(members, StatusCode::OK))
            }

            // 获取用户加入的群组列表
            (&Method::GET, "getUserGroups") => {
                let request = proto::group::GetUserGroupsRequest {
                    user_id: user_id.clone(),
                };

                let response = client.get_user_groups(request).await?;
                let inner = response.into_inner();
                let groups = inner.groups.iter().map(|g| Self::convert_user_group_to_json(g)).collect::<Vec<_>>();

                Ok(success_response(groups, StatusCode::OK))
            }

            // 检查用户是否在群组中
            (&Method::GET, "checkMembership") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::CheckMembershipRequest {
                    group_id: group_id.clone(),
                    user_id: user_id.clone(),
                };

                let response = client.check_membership(request).await?;
                let inner = response.into_inner();

                let role_text = if inner.is_member {
                    match inner.role.unwrap_or(proto::group::MemberRole::Member as i32) {
                        0 => "MEMBER",
                        1 => "ADMIN", 
                        2 => "OWNER",
                        _ => "UNKNOWN"
                    }
                } else {
                    "NONE"
                };

                Ok(success_response(
                    json!({
                        "isMember": inner.is_member,
                        "role": inner.role,
                        "roleText": role_text
                    }),
                    StatusCode::OK
                ))
            }

            // ---------- 新增的群聊功能方法 ----------
            
            // 创建群公告
            (&Method::POST, "createAnnouncement") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let content = extract_string_param(&body, "content", None)?;
                let is_pinned = get_bool_param(&body, "isPinned", Some("is_pinned"), false);

                let request = proto::group::CreateAnnouncementRequest {
                    group_id: group_id.clone(),
                    creator_id: user_id.clone(),
                    title: title.clone(),
                    content: content.clone(),
                    is_pinned,
                };

                let response = client.create_announcement(request).await?;
                let inner = response.into_inner();
                let announcement = inner.announcement.ok_or_else(|| anyhow::anyhow!("公告数据为空"))?;

                Ok(success_response(Self::convert_announcement_to_json(&announcement), StatusCode::OK))
            }

            // 获取群公告
            (&Method::GET, "getAnnouncement") => {
                let announcement_id = extract_string_param(&body, "announcementId", Some("announcement_id"))?;

                let request = proto::group::GetAnnouncementRequest {
                    announcement_id: announcement_id.clone(),
                };

                let response = client.get_announcement(request).await?;
                let inner = response.into_inner();
                let announcement = inner.announcement.ok_or_else(|| anyhow::anyhow!("公告数据为空"))?;

                Ok(success_response(Self::convert_announcement_to_json(&announcement), StatusCode::OK))
            }

            // 获取群组所有公告
            (&Method::GET, "getGroupAnnouncements") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetGroupAnnouncementsRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_group_announcements(request).await?;
                let inner = response.into_inner();
                let announcements = inner.announcements.iter()
                    .map(|a| Self::convert_announcement_to_json(a))
                    .collect::<Vec<_>>();

                Ok(success_response(announcements, StatusCode::OK))
            }

            // 删除群公告
            (&Method::GET, "deleteAnnouncement") => {
                let announcement_id = extract_string_param(&body, "announcementId", Some("announcement_id"))?;

                let request = proto::group::DeleteAnnouncementRequest {
                    announcement_id: announcement_id.clone(),
                    deleted_by_id: user_id.clone(),
                };

                let response = client.delete_announcement(request).await?;
                let inner = response.into_inner();

                Ok(success_response(
                    inner.success,
                    StatusCode::OK
                ))
            }

            // 获取群组设置
            (&Method::GET, "getGroupSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetGroupSettingsRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_group_settings(request).await?;
                let inner = response.into_inner();
                let settings = inner.settings.ok_or_else(|| anyhow::anyhow!("群组设置数据为空"))?;

                Ok(success_response(Self::convert_group_settings_to_json(&settings), StatusCode::OK))
            }

            // 更新群组设置
            (&Method::POST, "updateGroupSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let allow_member_friendship = get_bool_param(&body, "allowMemberFriendship", Some("allow_member_friendship"), true);
                let join_approval_required = get_bool_param(&body, "joinApprovalRequired", Some("join_approval_required"), false);
                let only_admin_can_invite = get_bool_param(&body, "onlyAdminCanInvite", Some("only_admin_can_invite"), false);
                let only_admin_can_modify = get_bool_param(&body, "onlyAdminCanModify", Some("only_admin_can_modify"), false);

                let request = proto::group::UpdateGroupSettingsRequest {
                    group_id: group_id.clone(),
                    updated_by_id: user_id.clone(),
                    allow_member_friendship,
                    join_approval_required,
                    only_admin_can_invite,
                    only_admin_can_modify,
                };

                let response = client.update_group_settings(request).await?;
                let inner = response.into_inner();
                let settings = inner.settings.ok_or_else(|| anyhow::anyhow!("群组设置数据为空"))?;

                Ok(success_response(Self::convert_group_settings_to_json(&settings), StatusCode::OK))
            }

            // 获取群组黑名单
            (&Method::GET, "getBlacklist") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetBlacklistRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_blacklist(request).await?;
                let inner = response.into_inner();
                let entries = inner.entries.iter()
                    .map(|e| Self::convert_blacklist_entry_to_json(e))
                    .collect::<Vec<_>>();

                Ok(success_response(entries, StatusCode::OK))
            }

            // 添加用户到黑名单
            (&Method::POST, "addToBlacklist") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let reason = body.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();

                let request = proto::group::AddToBlacklistRequest {
                    group_id: group_id.clone(),
                    user_id: target_user_id.clone(),
                    creator_id: user_id.clone(),
                    reason: reason.clone(),
                };

                let response = client.add_to_blacklist(request).await?;
                let inner = response.into_inner();
                let entry = inner.entry.ok_or_else(|| anyhow::anyhow!("黑名单数据为空"))?;

                Ok(success_response(Self::convert_blacklist_entry_to_json(&entry), StatusCode::OK))
            }

            // 从黑名单移除用户
            (&Method::POST, "removeFromBlacklist") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let request = proto::group::RemoveFromBlacklistRequest {
                    group_id: group_id.clone(),
                    user_id: target_user_id.clone(),
                    removed_by_id: user_id.clone(),
                };

                let response = client.remove_from_blacklist(request).await?;
                let inner = response.into_inner();

                Ok(success_response(
                    inner.success,
                    StatusCode::OK
                ))
            }

            // 禁言成员
            (&Method::POST, "muteMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let reason = body.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let is_permanent = get_bool_param(&body, "isPermanent", Some("is_permanent"), false);
                
                // 处理禁言截止时间
                let mute_until = if is_permanent {
                    // 永久禁言使用一个很远的未来时间
                    let future_time = SystemTime::now() + StdDuration::from_secs(100 * 365 * 24 * 3600); // 100年后
                    prost_types::Timestamp::from(future_time)
                } else {
                    let duration_minutes = body.get("durationMinutes")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(60); // 默认60分钟
                    
                    let now = SystemTime::now();
                    let future_time = now + StdDuration::from_secs((duration_minutes * 60) as u64);
                    prost_types::Timestamp::from(future_time)
                };

                let request = proto::group::MuteMemberRequest {
                    group_id: group_id.clone(),
                    user_id: target_user_id.clone(),
                    creator_id: user_id.clone(),
                    reason: reason.clone(),
                    mute_until: Some(mute_until),
                    is_permanent,
                };

                let response = client.mute_member(request).await?;
                let inner = response.into_inner();
                let entry = inner.entry.ok_or_else(|| anyhow::anyhow!("禁言数据为空"))?;

                Ok(success_response(Self::convert_mute_entry_to_json(&entry), StatusCode::OK))
            }

            // 解除成员禁言
            (&Method::POST, "unmuteMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let request = proto::group::UnmuteMemberRequest {
                    group_id: group_id.clone(),
                    user_id: target_user_id.clone(),
                    unmuted_by_id: user_id.clone(),
                };

                let response = client.unmute_member(request).await?;
                let inner = response.into_inner();

                Ok(success_response(
                    inner.success,
                    StatusCode::OK
                ))
            }

            // 获取被禁言的成员
            (&Method::GET, "getMutedMembers") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetMutedMembersRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_muted_members(request).await?;
                let inner = response.into_inner();
                let entries = inner.entries.iter()
                    .map(|e| Self::convert_mute_entry_to_json(e))
                    .collect::<Vec<_>>();

                Ok(success_response(entries, StatusCode::OK))
            }

            // 获取成员设置
            (&Method::GET, "getMemberSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = body.get("userId")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&user_id)
                    .to_string();

                let request = proto::group::GetMemberSettingsRequest {
                    group_id: group_id.clone(),
                    user_id: target_user_id.clone(),
                };

                let response = client.get_member_settings(request).await?;
                let inner = response.into_inner();
                let settings = inner.settings.ok_or_else(|| anyhow::anyhow!("成员设置数据为空"))?;

                Ok(success_response(Self::convert_member_settings_to_json(&settings), StatusCode::OK))
            }

            // 更新成员设置
            (&Method::POST, "updateMemberSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let mute_notifications = get_bool_param(&body, "muteNotifications", Some("mute_notifications"), false);
                let nickname_in_group = body.get("nicknameInGroup")
                    .or_else(|| body.get("nickname_in_group"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let request = proto::group::UpdateMemberSettingsRequest {
                    group_id: group_id.clone(),
                    user_id: user_id.clone(),
                    mute_notifications,
                    nickname_in_group: nickname_in_group.clone(),
                };

                let response = client.update_member_settings(request).await?;
                let inner = response.into_inner();
                let settings = inner.settings.ok_or_else(|| anyhow::anyhow!("成员设置数据为空"))?;

                Ok(success_response(Self::convert_member_settings_to_json(&settings), StatusCode::OK))
            }

            // 创建群二维码
            (&Method::POST, "createGroupQrcode") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let is_permanent = get_bool_param(&body, "isPermanent", Some("is_permanent"), false);
                
                // 处理过期时间
                let expires_at = if is_permanent {
                    // 永久二维码使用一个很远的未来时间
                    let future_time = SystemTime::now() + StdDuration::from_secs(100 * 365 * 24 * 3600); // 100年后
                    prost_types::Timestamp::from(future_time)
                } else {
                    let valid_days = body.get("validDays")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(7); // 默认7天
                    
                    let now = SystemTime::now();
                    let future_time = now + StdDuration::from_secs((valid_days * 24 * 60 * 60) as u64);
                    prost_types::Timestamp::from(future_time)
                };

                let request = proto::group::CreateGroupQrcodeRequest {
                    group_id: group_id.clone(),
                    creator_id: user_id.clone(),
                    expires_at: Some(expires_at),
                    is_permanent,
                };

                let response = client.create_group_qrcode(request).await?;
                let inner = response.into_inner();
                let qrcode = inner.qrcode.ok_or_else(|| anyhow::anyhow!("二维码数据为空"))?;

                Ok(success_response(Self::convert_qrcode_to_json(&qrcode), StatusCode::OK))
            }

            // 获取群二维码
            (&Method::GET, "getGroupQrcode") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::group::GetGroupQrcodeRequest {
                    group_id: group_id.clone(),
                };

                let response = client.get_group_qrcode(request).await?;
                let inner = response.into_inner();
                let qrcode = inner.qrcode.ok_or_else(|| anyhow::anyhow!("二维码数据为空"))?;

                Ok(success_response(Self::convert_qrcode_to_json(&qrcode), StatusCode::OK))
            }

            // 其他未实现的方法
            _ => {
                error!("群组服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("群组服务不支持的方法: {}", method_name))
            }
        }
    }

    /// 将群组消息转换为JSON
    fn convert_group_to_json(group: &proto::group::Group) -> Value {
        json!({
            "id": group.id,
            "name": group.name,
            "description": group.description,
            "avatarUrl": group.avatar_url,
            "ownerId": group.owner_id,
            "memberCount": group.member_count,
            "createdAt": timestamp_to_datetime_string(&group.created_at),
            "updatedAt": timestamp_to_datetime_string(&group.updated_at),
        })
    }

    /// 将群组成员消息转换为JSON
    fn convert_member_to_json(member: &proto::group::Member) -> Value {
        let role_text = match member.role {
            0 => "MEMBER",
            1 => "ADMIN",
            2 => "OWNER",
            _ => "UNKNOWN"
        };

        json!({
            "id": member.id,
            "groupId": member.group_id,
            "userId": member.user_id,
            "username": member.username,
            "nickname": member.nickname,
            "avatarUrl": member.avatar_url,
            "role": member.role,
            "roleText": role_text,
            "joinedAt": timestamp_to_datetime_string(&member.joined_at),
        })
    }

    /// 将用户群组消息转换为JSON
    fn convert_user_group_to_json(user_group: &proto::group::UserGroup) -> Value {
        let role_text = match user_group.role {
            0 => "MEMBER",
            1 => "ADMIN",
            2 => "OWNER",
            _ => "UNKNOWN"
        };

        json!({
            "id": user_group.id,
            "name": user_group.name,
            "avatarUrl": user_group.avatar_url,
            "memberCount": user_group.member_count,
            "role": user_group.role,
            "roleText": role_text,
            "joinedAt": timestamp_to_datetime_string(&user_group.joined_at),
        })
    }

    /// 将群公告消息转换为JSON
    fn convert_announcement_to_json(announcement: &proto::group::Announcement) -> Value {
        json!({
            "id": announcement.id,
            "groupId": announcement.group_id,
            "creatorId": announcement.creator_id,
            "title": announcement.title,
            "content": announcement.content,
            "isPinned": announcement.is_pinned,
            "createdAt": timestamp_to_datetime_string(&announcement.created_at),
            "updatedAt": timestamp_to_datetime_string(&announcement.updated_at),
        })
    }

    /// 将群组设置消息转换为JSON
    fn convert_group_settings_to_json(settings: &proto::group::GroupSettings) -> Value {
        json!({
            "groupId": settings.group_id,
            "allowMemberFriendship": settings.allow_member_friendship,
            "joinApprovalRequired": settings.join_approval_required,
            "onlyAdminCanInvite": settings.only_admin_can_invite,
            "onlyAdminCanModify": settings.only_admin_can_modify,
        })
    }

    /// 将黑名单条目转换为JSON
    fn convert_blacklist_entry_to_json(entry: &proto::group::BlacklistEntry) -> Value {
        json!({
            "id": entry.id,
            "groupId": entry.group_id,
            "userId": entry.user_id,
            "creatorId": entry.creator_id,
            "reason": entry.reason,
            "createdAt": timestamp_to_datetime_string(&entry.created_at),
        })
    }

    /// 将禁言条目转换为JSON
    fn convert_mute_entry_to_json(entry: &proto::group::MuteEntry) -> Value {
        let mute_until = timestamp_to_datetime_string(&entry.mute_until);
        let now = chrono::Utc::now().to_rfc3339();

        json!({
            "id": entry.id,
            "groupId": entry.group_id,
            "userId": entry.user_id,
            "creatorId": entry.creator_id,
            "reason": entry.reason,
            "muteUntil": mute_until,
            "isPermanent": entry.is_permanent,
            "isActive": entry.is_permanent || mute_until > now,
            "createdAt": timestamp_to_datetime_string(&entry.created_at),
            "updatedAt": timestamp_to_datetime_string(&entry.updated_at),
        })
    }

    /// 将成员设置转换为JSON
    fn convert_member_settings_to_json(settings: &proto::group::MemberSettings) -> Value {
        json!({
            "id": settings.id,
            "groupId": settings.group_id,
            "userId": settings.user_id,
            "muteNotifications": settings.mute_notifications,
            "nicknameInGroup": settings.nickname_in_group,
            "createdAt": timestamp_to_datetime_string(&settings.created_at),
            "updatedAt": timestamp_to_datetime_string(&settings.updated_at),
        })
    }

    /// 将群二维码转换为JSON
    fn convert_qrcode_to_json(qrcode: &proto::group::GroupQrcode) -> Value {
        json!({
            "id": qrcode.id,
            "groupId": qrcode.group_id,
            "creatorId": qrcode.creator_id,
            "qrcodeUrl": qrcode.qrcode_url,
            "expiresAt": timestamp_to_datetime_string(&qrcode.expires_at),
            "isPermanent": qrcode.is_permanent,
            "createdAt": timestamp_to_datetime_string(&qrcode.created_at),
        })
    }
} 