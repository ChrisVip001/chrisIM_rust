use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::grpc_client::{UserServiceGrpcClient, FriendServiceGrpcClient, GroupServiceGrpcClient};
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug};

use super::common::{
    success_response, extract_string_param, get_optional_string, 
    get_i64_param, timestamp_to_datetime_string, get_user_id_from_jwt,
    format_timestamp,
};
use crate::auth::jwt::UserInfo;

/// 通用服务处理器，整合用户、好友和群组服务
#[derive(Clone)]
pub struct CommonServiceHandler {
    user_client: UserServiceGrpcClient,
    friend_client: FriendServiceGrpcClient,
    group_client: GroupServiceGrpcClient,
}

impl CommonServiceHandler {
    /// 创建新的通用服务处理器
    pub fn new(
        user_client: UserServiceGrpcClient,
        friend_client: FriendServiceGrpcClient,
        group_client: GroupServiceGrpcClient,
    ) -> Self {
        Self { user_client, friend_client, group_client }
    }

    /// 处理通用服务请求
    pub async fn handle_request(
        &mut self,
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
            // 模糊查询当前用户的好友和群聊
            (&Method::POST, "searchFriendsAndGroups") => {
                let keyword = get_optional_string(&body, "keyword", None).unwrap_or_default();
                
                // 如果关键字为空，直接返回空数组
                if keyword.trim().is_empty() {
                    return Ok(success_response(json!({
                        "friends": [],
                        "groups": []
                    }), StatusCode::OK));
                }
                
                // 使用friend_client的get_friend_list_with_params接口搜索好友
                // 该接口支持关键字搜索且在数据库层面执行
                let friends_response = self.friend_client.get_friend_list_with_params(
                    &user_id,
                    1,  // 第一页
                    5000, // 每页5000条
                    "", // 默认排序
                    &keyword
                ).await?;
                
                // 转换好友列表
                let friends = friends_response.friends.iter()
                    .map(|f| self.convert_friend_to_json(f))
                    .collect::<Vec<_>>();
                
                // 使用search_user_groups在服务端搜索群组
                let groups_response = self.group_client.search_user_groups(
                    &user_id,
                    &keyword,
                    1,  // 第一页
                    5000  // 每页5000条
                ).await?;
                
                // 转换群组列表
                let groups = groups_response.groups.iter()
                    .map(|g| self.convert_user_group_to_json(g))
                    .collect::<Vec<_>>();
                
                // 返回分类结果
                Ok(success_response(json!({
                    "friends": friends,
                    "groups": groups
                }), StatusCode::OK))
            },
            
          
          
            
            // 其他未实现的方法
            _ => {
                error!("通用服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("通用服务不支持的方法: {}", method_name))
            }
        }
    }
    
    // ===== 转换方法，将不同服务的数据转换为JSON格式 =====
    
    /// 将详细好友信息转换为JSON
    fn convert_detailed_friend_to_json(&self, friend: &proto::friend::DetailedFriend) -> Value {
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
    fn convert_user_group_to_json(&self, group: &proto::group::UserGroup) -> Value {
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
    fn convert_friend_to_json(&self, friend: &proto::friend::Friend) -> Value {
        json!({
            "id": friend.id,
            "username": friend.username,
            "nickname": friend.nickname,
            "avatarUrl": friend.avatar_url,
            "friendshipCreatedAt": timestamp_to_datetime_string(&friend.friendship_created_at),
            "remark": friend.remark,
        })
    }
} 