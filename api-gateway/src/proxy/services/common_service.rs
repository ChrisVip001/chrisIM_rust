use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::proto;
use serde_json::{json, Value};
use tonic::transport::Channel;
use tracing::{error, debug};
use common::proto::friend::friend_service_client::FriendServiceClient;
use common::proto::friend::GetFriendListRequest;
use common::proto::group::group_service_client::GroupServiceClient;
use common::proto::group::SearchUserGroupsRequest;
use common::proto::user::user_service_client::UserServiceClient;
use super::common::{
    success_response, extract_string_param, get_optional_string, 
    get_i64_param, timestamp_to_datetime_string, get_user_id_from_jwt,
    format_timestamp,
};
use crate::auth::jwt::UserInfo;

/// 通用服务处理器，整合用户、好友和群组服务
#[derive(Clone)]
pub struct CommonServiceHandler;

impl CommonServiceHandler {

    /// 处理通用服务请求
    pub async fn handle_request(
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<UserInfo>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理通用服务请求: {} {}", method, path);

        // 从JWT中获取用户ID
        let user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;

        // 从路径提取方法名 - 格式: /api/common/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 模糊查询当前用户的好友和群聊（并行调用）
            (&Method::POST, "searchFriendsAndGroups") => {
                let keyword = get_optional_string(&body, "keyword", None).unwrap_or_default();

                // 如果关键字为空，直接返回空数组
                if keyword.trim().is_empty() {
                    return Ok(success_response(json!({
                        "friends": [],
                        "groups": []
                    }), StatusCode::OK));
                }

                // 并行调用好友服务和群组服务
                let (friends_result, groups_result) = tokio::join!(
                    Self::search_friends(&user_id, &keyword),
                    Self::search_groups(&user_id, &keyword)
                );

                // 处理结果
                let friends = friends_result.unwrap_or_else(|e| {
                    error!("搜索好友失败: {}", e);
                    Vec::new() // 如果好友搜索失败，返回空数组而不是整个请求失败
                });

                let groups = groups_result.unwrap_or_else(|e| {
                    error!("搜索群组失败: {}", e);
                    Vec::new() // 如果群组搜索失败，返回空数组而不是整个请求失败
                });

                // 返回分类结果
                Ok(success_response(json!({
                    "friends": friends,
                    "groups": groups
                }), StatusCode::OK))
            },
            
            // 获取用户仪表板信息（并行获取用户信息、好友列表、群组列表）
            (&Method::GET, "getUserDashboard") => {
                // 并行调用三个服务
                let (user_result, friends_result, groups_result) = tokio::join!(
                    Self::get_user_info(&user_id),
                    Self::search_friends(&user_id, ""), // 空关键词表示获取所有好友
                    Self::search_groups(&user_id, "")   // 空关键词表示获取所有群组
                );

                // 处理结果，失败时返回默认值而不是让整个请求失败
                let user_info = user_result.unwrap_or_else(|e| {
                    error!("获取用户信息失败: {}", e);
                    json!({"error": "获取用户信息失败"})
                });

                let friends = friends_result.unwrap_or_else(|e| {
                    error!("获取好友列表失败: {}", e);
                    Vec::new()
                });

                let groups = groups_result.unwrap_or_else(|e| {
                    error!("获取群组列表失败: {}", e);
                    Vec::new()
                });

                // 返回仪表板数据
                Ok(success_response(json!({
                    "user": user_info,
                    "friends": friends,
                    "groups": groups,
                    "summary": {
                        "friendCount": friends.len(),
                        "groupCount": groups.len()
                    }
                }), StatusCode::OK))
            },
            
            // 其他未实现的方法
            _ => {
                error!("通用服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("通用服务不支持的方法: {}", method_name))
            }
        }
    }
    
    /// 获取用户信息（独立的异步方法）
    async fn get_user_info(user_id: &str) -> anyhow::Result<Value> {
        let mut user_client = common::service::user_client().await?;

        let request = proto::user::GetUserByIdRequest {
            user_id: user_id.to_string(),
        };

        let user_response = user_client.get_user_by_id(request).await?;
        let user = user_response.into_inner().user
            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

        Ok(Self::convert_user_to_json(&user))
    }

    /// 搜索好友（独立的异步方法）
    async fn search_friends(user_id: &str, keyword: &str) -> anyhow::Result<Vec<Value>> {
        let mut friend_client = common::service::friend_client().await?;

        let request = GetFriendListRequest {
            user_id: user_id.to_string(),
            page: 1,
            page_size: 5000,
            sort_by: String::new(),
            keyword: keyword.to_string(),
        };

        let friends_response = friend_client.get_friend_list(request).await?;
        let friends = friends_response.into_inner().friends
            .iter()
            .map(|f| Self::convert_friend_to_json(f))
            .collect();

        Ok(friends)
    }

    /// 搜索群组（独立的异步方法）
    async fn search_groups(user_id: &str, keyword: &str) -> anyhow::Result<Vec<Value>> {
        let mut group_client = common::service::group_client().await?;

        let request = SearchUserGroupsRequest {
            user_id: user_id.to_string(),
            keyword: keyword.to_string(),
            page: 1,
            page_size: 5000,
        };

        let groups_response = group_client.search_user_groups(request).await?;
        let groups = groups_response.into_inner().groups
            .iter()
            .map(|g| Self::convert_user_group_to_json(g))
            .collect();

        Ok(groups)
    }

    // ===== 转换方法，将不同服务的数据转换为JSON格式 =====
    
    /// 将详细好友信息转换为JSON
    fn convert_detailed_friend_to_json(friend: &proto::friend::DetailedFriend) -> Value {
        // 转换好友类型为文本
        let friend_type_text = match friend.friend_type {
            0 => "friend",     // 普通好友
            1 => "official",   // 官方账号
            2 => "system",     // 系统账号
            _ => "friend",     // 默认普通好友
        };
        
        json!({
            "id": friend.id,
            "username": friend.username,
            "nickname": friend.nickname,
            "avatarUrl": friend.avatar_url,
            "friendshipCreatedAt": timestamp_to_datetime_string(&friend.friendship_created_at),
            "remark": friend.remark,
            "isOnline": friend.is_online,
            "isStarred": friend.is_starred,
            "isTop": friend.is_top,
            "relationStatus": friend.relation_status,
            "friendType": friend_type_text
        })
    }
    
    /// 将用户群组转换为JSON
    fn convert_user_group_to_json(group: &proto::group::UserGroup) -> Value {
        json!({
            "id": group.id,
            "name": group.name,
            "avatarUrl": group.avatar_url,
            "memberCount": group.member_count,
            "role": group.role,
            "roleText": match group.role {
                0 => "MEMBER",
                1 => "ADMIN",
                2 => "OWNER",
                _ => "UNKNOWN"
            },
            // "joinTime": timestamp_to_datetime_string(&group.join_time),
            // "lastActiveTime": timestamp_to_datetime_string(&group.last_active_time),
            // "unreadCount": group.unread_count,
        })
    }
    
    /// 将好友信息转换为JSON
    fn convert_friend_to_json(friend: &proto::friend::Friend) -> Value {
        json!({
            "id": friend.id,
            "username": friend.username,
            "nickname": friend.nickname,
            "avatarUrl": friend.avatar_url,
            "friendshipCreatedAt": timestamp_to_datetime_string(&friend.friendship_created_at),
            "remark": friend.remark,
        })
    }

    /// 将用户信息转换为JSON
    fn convert_user_to_json(user: &proto::user::User) -> Value {
        json!({
            "id": user.id,
            "username": user.username,
            "email": user.email,
            "nickname": user.nickname,
            "avatarUrl": user.avatar_url,
            "phone": user.phone,
            "address": user.address,
            "headImage": user.head_image,
            "headImageThumb": user.head_image_thumb,
            "sex": user.sex,
            "userStat": user.user_stat,
            "tenantId": user.tenant_id,
            "customId": user.custom_id,
            "createdAt": timestamp_to_datetime_string(&user.created_at),
            "updatedAt": timestamp_to_datetime_string(&user.updated_at),
            "lastLoginTime": timestamp_to_datetime_string(&user.last_login_time),
        })
    }
}