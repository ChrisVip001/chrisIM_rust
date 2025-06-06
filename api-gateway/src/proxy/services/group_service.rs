use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::grpc_client::GroupServiceGrpcClient;
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug};
use chrono::{DateTime, TimeZone, Utc};
use std::time::{Duration as StdDuration, SystemTime};

use super::common::{
    success_response, extract_string_param, get_optional_string, 
    get_i64_param, timestamp_to_datetime_string, get_user_id_from_jwt,
    get_bool_param,get_option_bool_param,
};
use crate::auth::jwt::UserInfo;

/// 群组服务处理器
#[derive(Clone)]
pub struct GroupServiceHandler {
    client: GroupServiceGrpcClient,
}

impl GroupServiceHandler {
    /// 创建新的群组服务处理器
    pub fn new(client: GroupServiceGrpcClient) -> Self {
        Self { client }
    }

    /// 处理群组服务请求
    pub async fn handle_request(
        &mut self,
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<UserInfo>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理群组服务请求: {} {}", method, path);

        // 从JWT中获取用户ID
        let current_user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;

        // 从路径提取方法名 - 格式: /api/groups/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 创建群组
            (&Method::POST, "create") => {
                let name = extract_string_param(&body, "name", None)?;
                
                let description = get_optional_string(&body,"description",None).unwrap_or_default();
                
                let avatar_url = get_optional_string(&body,"avatarUrl",Some("avatar_url")).unwrap_or_default();

                // 处理初始成员列表
                let mut members = Vec::new();
                if let Some(member_ids) = body.get("members").and_then(|v| v.as_array()) {
                    for member_id in member_ids {
                        if let Some(member_user_id) = member_id.as_str() {
                            members.push(member_user_id.to_string());
                        }
                    }
                }

                let response = self.client.create_group(
                    &name,
                    &description,
                    &current_user_id,
                    &avatar_url,
                    members
                ).await?;

                let group = response.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(self.convert_group_to_json(&group), StatusCode::OK))
            }

            // 获取群组信息
            (&Method::GET, "getInfo") | (&Method::GET, "get") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_group(&group_id).await?;
                let group = response.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(self.convert_group_to_json(&group), StatusCode::OK))
            }

            // 更新群组信息
            (&Method::POST, "update") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                
                let name = get_optional_string(&body, "name", None);
                let description = get_optional_string(&body, "description", None);
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url"));

                let response = self.client.update_group(
                    &group_id,
                    name,
                    description,
                    avatar_url
                ).await?;
                
                let group = response.group.ok_or_else(|| anyhow::anyhow!("群组数据为空"))?;

                Ok(success_response(self.convert_group_to_json(&group), StatusCode::OK))
            }

            // 删除群组
            (&Method::GET, "delete") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.delete_group(&group_id, &current_user_id).await?;

                Ok(success_response(
                    response.success,
                    StatusCode::OK,
                ))
            }

            // 添加成员
            (&Method::POST, "addMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                
                // 处理成员ID列表
                let mut members = Vec::new();
                if let Some(member_ids) = body.get("members").and_then(|v| v.as_array()) {
                    for member_id in member_ids {
                        if let Some(member_user_id) = member_id.as_str() {
                            members.push(member_user_id.to_string());
                        }
                    }
                } else if let Some(user_id) = body.get("userId").and_then(|v| v.as_str()) {
                    // 兼容单个用户ID的旧接口
                    members.push(user_id.to_string());
                }
                
                // 确保有成员要添加
                if members.is_empty() {
                    return Err(anyhow::anyhow!("没有指定要添加的成员"));
                }

                // 固定使用普通成员角色
                let role = proto::group::MemberRole::Member;

                let response = self.client.add_member(&group_id, members, &current_user_id, role).await?;
                
                Ok(success_response(
                    json!({
                        "success": response.success,
                        "addedCount": response.added_count
                    }),
                    StatusCode::OK
                ))
            }

            // 移除成员
            (&Method::POST, "removeMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                
                // 处理成员ID列表
                let mut members = Vec::new();
                if let Some(member_ids) = body.get("members").and_then(|v| v.as_array()) {
                    for member_id in member_ids {
                        if let Some(member_user_id) = member_id.as_str() {
                            members.push(member_user_id.to_string());
                        }
                    }
                } else if let Some(user_id) = body.get("userId").and_then(|v| v.as_str()) {
                    // 兼容单个用户ID的旧接口
                    members.push(user_id.to_string());
                }
                
                // 确保有成员要移除
                if members.is_empty() {
                    return Err(anyhow::anyhow!("没有指定要移除的成员"));
                }

                let response = self.client.remove_member(&group_id, members, &current_user_id).await?;
                
                Ok(success_response(
                    response.success,
                    StatusCode::OK
                ))
            }

            // 更新成员角色
            (&Method::POST, "updateMemberRole") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let member_id = extract_string_param(&body, "userId", Some("user_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                let response = self.client.update_member_role(&group_id, &member_id, &current_user_id, role).await?;
                let member = response.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(self.convert_member_to_json(&member), StatusCode::OK))
            }

            // 获取群组成员列表
            (&Method::GET, "getMembers") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let page = get_i64_param(&body, "page", 1) as i32;
                let page_size = get_i64_param(&body, "pageSize", 20) as i32;

                let response = self.client.get_members_with_params(
                    &group_id,
                    page,
                    page_size
                ).await?;
                
                let members = response.members.iter().map(|m| self.convert_member_to_json(m)).collect::<Vec<_>>();

                Ok(success_response(
                    json!({
                        "members": members,
                        "total": response.total,
                        "page": response.page,
                        "pageSize": response.page_size
                    }), 
                    StatusCode::OK
                ))
            }

            // 获取用户加入的群组列表
            (&Method::GET, "getUserGroups") => {
                let response = self.client.get_user_groups(&current_user_id).await?;
                let groups = response.groups.iter().map(|g| self.convert_user_group_to_json(g)).collect::<Vec<_>>();

                Ok(success_response(groups, StatusCode::OK))
            }

            // 检查用户是否在群组中
            (&Method::GET, "checkMembership") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.check_membership(&group_id, &current_user_id).await?;

                let role_text = if response.is_member {
                    match response.role.unwrap_or(0) {
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
                        "isMember": response.is_member,
                        "role": response.role,
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

                let response = self.client.create_announcement(
                    &group_id,
                    &current_user_id,
                    &title,
                    &content,
                    is_pinned
                ).await?;

                let announcement = response.announcement.ok_or_else(|| anyhow::anyhow!("公告数据为空"))?;

                Ok(success_response(self.convert_announcement_to_json(&announcement), StatusCode::OK))
            }

            // 获取群公告
            (&Method::GET, "getAnnouncement") => {
                let announcement_id = extract_string_param(&body, "announcementId", Some("announcement_id"))?;

                let response = self.client.get_announcement(&announcement_id).await?;
                let announcement = response.announcement.ok_or_else(|| anyhow::anyhow!("公告数据为空"))?;

                Ok(success_response(self.convert_announcement_to_json(&announcement), StatusCode::OK))
            }

            // 获取群组所有公告
            (&Method::GET, "getGroupAnnouncements") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_group_announcements(&group_id).await?;
                let announcements = response.announcements.iter()
                    .map(|a| self.convert_announcement_to_json(a))
                    .collect::<Vec<_>>();

                Ok(success_response(announcements, StatusCode::OK))
            }

            // 删除群公告
            (&Method::GET, "deleteAnnouncement") => {
                let announcement_id = extract_string_param(&body, "announcementId", Some("announcement_id"))?;

                let response = self.client.delete_announcement(&announcement_id, &current_user_id).await?;

                Ok(success_response(
                    response.success,
                    StatusCode::OK
                ))
            }

            // 获取群组设置
            (&Method::GET, "getGroupSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_group_settings(&group_id).await?;
                let settings = response.settings.ok_or_else(|| anyhow::anyhow!("群组设置数据为空"))?;

                Ok(success_response(self.convert_group_settings_to_json(&settings), StatusCode::OK))
            }

            // 更新群组设置
            (&Method::POST, "updateGroupSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let allow_member_friendship = get_bool_param(&body, "allowMemberFriendship", Some("allow_member_friendship"), true);
                let join_approval_required = get_bool_param(&body, "joinApprovalRequired", Some("join_approval_required"), false);
                let only_admin_can_invite = get_bool_param(&body, "onlyAdminCanInvite", Some("only_admin_can_invite"), false);
                let only_admin_can_modify = get_bool_param(&body, "onlyAdminCanModify", Some("only_admin_can_modify"), false);

                let response = self.client.update_group_settings(
                    &group_id,
                    &current_user_id,
                    allow_member_friendship,
                    join_approval_required,
                    only_admin_can_invite,
                    only_admin_can_modify
                ).await?;

                let settings = response.settings.ok_or_else(|| anyhow::anyhow!("群组设置数据为空"))?;

                Ok(success_response(self.convert_group_settings_to_json(&settings), StatusCode::OK))
            }

            // 获取群组黑名单
            (&Method::GET, "getBlacklist") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_blacklist(&group_id).await?;
                let entries = response.entries.iter()
                    .map(|e| self.convert_blacklist_entry_to_json(e))
                    .collect::<Vec<_>>();

                Ok(success_response(entries, StatusCode::OK))
            }

            // 添加用户到黑名单
            (&Method::POST, "addToBlacklist") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let reason = body.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();

                let response = self.client.add_to_blacklist(
                    &group_id,
                    &target_user_id,
                    &current_user_id,
                    &reason
                ).await?;

                let entry = response.entry.ok_or_else(|| anyhow::anyhow!("黑名单数据为空"))?;

                Ok(success_response(self.convert_blacklist_entry_to_json(&entry), StatusCode::OK))
            }

            // 从黑名单移除用户
            (&Method::POST, "removeFromBlacklist") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.client.remove_from_blacklist(
                    &group_id,
                    &target_user_id,
                    &current_user_id
                ).await?;

                Ok(success_response(
                    response.success,
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
                    None
                } else {
                    let duration_minutes = body.get("durationMinutes")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(60); // 默认60分钟
                    
                    let now = SystemTime::now();
                    let future_time = now + StdDuration::from_secs((duration_minutes * 60) as u64);
                    Some(prost_types::Timestamp::from(future_time))
                };

                let response = self.client.mute_member(
                    &group_id,
                    &target_user_id,
                    &current_user_id,
                    &reason,
                    mute_until,
                    is_permanent
                ).await?;

                let entry = response.entry.ok_or_else(|| anyhow::anyhow!("禁言数据为空"))?;

                Ok(success_response(self.convert_mute_entry_to_json(&entry), StatusCode::OK))
            }

            // 解除成员禁言
            (&Method::POST, "unmuteMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.client.unmute_member(
                    &group_id,
                    &target_user_id,
                    &current_user_id
                ).await?;

                Ok(success_response(
                    response.success,
                    StatusCode::OK
                ))
            }

            // 获取被禁言的成员
            (&Method::GET, "getMutedMembers") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_muted_members(&group_id).await?;
                let entries = response.entries.iter()
                    .map(|e| self.convert_mute_entry_to_json(e))
                    .collect::<Vec<_>>();

                Ok(success_response(entries, StatusCode::OK))
            }

            // 获取成员设置
            (&Method::POST, "getMemberSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let target_user_id = body.get("userId")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&current_user_id)
                    .to_string();

                let response = self.client.get_member_settings(&group_id, &target_user_id).await?;
                let settings = response.settings.ok_or_else(|| anyhow::anyhow!("成员设置数据为空"))?;

                Ok(success_response(self.convert_member_settings_to_json(&settings), StatusCode::OK))
            }

            // 更新成员设置
            (&Method::POST, "updateMemberSettings") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let mute_notifications = get_option_bool_param(&body, "muteNotifications", Some("mute_notifications"));
                let is_top = get_option_bool_param(&body, "isTop", Some("is_top"));
                let recall_notification = get_option_bool_param(&body, "recallNotification", Some("recall_notification"));
                let show_nickname = get_option_bool_param(&body, "showNickname", Some("show_nickname"));
                let nickname_in_group = get_optional_string(&body, "nicknameInGroup", Some("nickname_in_group"));
                let remark = get_optional_string(&body, "remark", Some("remark"));

                
                // 直接使用可选参数调用客户端方法
                let response = self.client.update_member_settings(
                    &group_id,
                    &current_user_id,
                    mute_notifications,
                    nickname_in_group,
                    remark,
                    is_top,
                    recall_notification,
                    show_nickname
                ).await?;

                let settings = response.settings.ok_or_else(|| anyhow::anyhow!("成员设置数据为空"))?;

                Ok(success_response(self.convert_member_settings_to_json(&settings), StatusCode::OK))
            }

            // 创建群二维码
            (&Method::POST, "createGroupQrcode") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let is_permanent = get_bool_param(&body, "isPermanent", Some("is_permanent"), false);
                
                // 处理过期时间
                let expires_at = if is_permanent {
                    None
                } else {
                    let valid_days = body.get("validDays")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(7); // 默认7天
                    
                    let now = SystemTime::now();
                    let future_time = now + StdDuration::from_secs((valid_days * 24 * 60 * 60) as u64);
                    Some(prost_types::Timestamp::from(future_time))
                };

                let response = self.client.create_group_qrcode(
                    &group_id,
                    &current_user_id,
                    expires_at,
                    is_permanent
                ).await?;

                let qrcode = response.qrcode.ok_or_else(|| anyhow::anyhow!("二维码数据为空"))?;

                Ok(success_response(self.convert_qrcode_to_json(&qrcode), StatusCode::OK))
            }

            // 获取群二维码
            (&Method::GET, "getGroupQrcode") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_group_qrcode(&group_id).await?;
                let qrcode = response.qrcode.ok_or_else(|| anyhow::anyhow!("二维码数据为空"))?;

                Ok(success_response(self.convert_qrcode_to_json(&qrcode), StatusCode::OK))
            }

            // 其他未实现的方法
            _ => {
                error!("群组服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("群组服务不支持的方法: {}", method_name))
            }
        }
    }

    /// 将群组消息转换为JSON
    fn convert_group_to_json(&self, group: &proto::group::Group) -> Value {
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
    fn convert_member_to_json(&self, member: &proto::group::Member) -> Value {
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
    fn convert_user_group_to_json(&self, user_group: &proto::group::UserGroup) -> Value {
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
    fn convert_announcement_to_json(&self, announcement: &proto::group::Announcement) -> Value {
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
    fn convert_group_settings_to_json(&self, settings: &proto::group::GroupSettings) -> Value {
        json!({
            "groupId": settings.group_id,
            "allowMemberFriendship": settings.allow_member_friendship,
            "joinApprovalRequired": settings.join_approval_required,
            "onlyAdminCanInvite": settings.only_admin_can_invite,
            "onlyAdminCanModify": settings.only_admin_can_modify,
        })
    }

    /// 将黑名单条目转换为JSON
    fn convert_blacklist_entry_to_json(&self, entry: &proto::group::BlacklistEntry) -> Value {
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
    fn convert_mute_entry_to_json(&self, entry: &proto::group::MuteEntry) -> Value {
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
    fn convert_member_settings_to_json(&self, settings: &proto::group::MemberSettings) -> Value {
        json!({
            "id": settings.id,
            "groupId": settings.group_id,
            "userId": settings.user_id,
            "muteNotifications": settings.mute_notifications,
            "nicknameInGroup": settings.nickname_in_group,
            "remark": settings.remark,
            "isTop": settings.is_top,
            "recallNotification": settings.recall_notification,
            "showNickname": settings.show_nickname,
            "createdAt": timestamp_to_datetime_string(&settings.created_at),
            "updatedAt": timestamp_to_datetime_string(&settings.updated_at),
        })
    }

    /// 将群二维码转换为JSON
    fn convert_qrcode_to_json(&self, qrcode: &proto::group::GroupQrcode) -> Value {
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