use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::grpc_client::GroupServiceGrpcClient;
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug};

use super::common::{
    success_response, extract_string_param, get_optional_string, 
    get_i64_param, timestamp_to_datetime_string,
};

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
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理群组服务请求: {} {}", method, path);

        // 从路径提取方法名 - 格式: /api/groups/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 创建群组
            (&Method::POST, "create") => {
                let name = extract_string_param(&body, "name", None)?;
                let owner_id = extract_string_param(&body, "ownerId", Some("owner_id"))?;
                
                let description = body.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                
                let avatar_url = body.get("avatarUrl")
                    .or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                // 处理初始成员列表
                let mut members = Vec::new();
                if let Some(member_ids) = body.get("members").and_then(|v| v.as_array()) {
                    for member_id in member_ids {
                        if let Some(user_id) = member_id.as_str() {
                            members.push(user_id.to_string());
                        }
                    }
                }

                let response = self.client.create_group(
                    &name,
                    description,
                    &owner_id,
                    avatar_url,
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
            (&Method::DELETE, "delete") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.client.delete_group(&group_id, &user_id).await?;

                Ok(success_response(
                    json!({"success": response.success}),
                    StatusCode::OK
                ))
            }

            // 添加成员
            (&Method::POST, "addMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let added_by_id = extract_string_param(&body, "addedById", Some("added_by_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                let response = self.client.add_member(&group_id, &user_id, &added_by_id, role).await?;
                let member = response.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(self.convert_member_to_json(&member), StatusCode::OK))
            }

            // 移除成员
            (&Method::DELETE, "removeMember") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let removed_by_id = extract_string_param(&body, "removedById", Some("removed_by_id"))?;

                let response = self.client.remove_member(&group_id, &user_id, &removed_by_id).await?;
                
                Ok(success_response(
                    json!({"success": response.success}),
                    StatusCode::OK
                ))
            }

            // 更新成员角色
            (&Method::PUT, "updateMemberRole") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let updated_by_id = extract_string_param(&body, "updatedById", Some("updated_by_id"))?;
                
                let role_value = get_i64_param(&body, "role", 0);
                let role = match role_value {
                    0 => proto::group::MemberRole::Member,
                    1 => proto::group::MemberRole::Admin,
                    2 => proto::group::MemberRole::Owner,
                    _ => proto::group::MemberRole::Member,
                };

                let response = self.client.update_member_role(&group_id, &user_id, &updated_by_id, role).await?;
                let member = response.member.ok_or_else(|| anyhow::anyhow!("成员数据为空"))?;

                Ok(success_response(self.convert_member_to_json(&member), StatusCode::OK))
            }

            // 获取群组成员列表
            (&Method::GET, "getMembers") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let response = self.client.get_members(&group_id).await?;
                let members = response.members.iter().map(|m| self.convert_member_to_json(m)).collect::<Vec<_>>();

                Ok(success_response(members, StatusCode::OK))
            }

            // 获取用户加入的群组列表
            (&Method::GET, "getUserGroups") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.client.get_user_groups(&user_id).await?;
                let groups = response.groups.iter().map(|g| self.convert_user_group_to_json(g)).collect::<Vec<_>>();

                Ok(success_response(groups, StatusCode::OK))
            }

            // 检查用户是否在群组中
            (&Method::GET, "checkMembership") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.client.check_membership(&group_id, &user_id).await?;

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
} 