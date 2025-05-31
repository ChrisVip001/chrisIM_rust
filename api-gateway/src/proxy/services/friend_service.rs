use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug};
use tonic::transport::Channel;
use common::proto::friend::friend_service_client::FriendServiceClient;
use super::common::{success_response,error_response, extract_string_param, timestamp_to_datetime_string,
                    get_i64_param, get_optional_string, get_user_id_from_jwt};
use crate::auth::jwt::UserInfo;

/// 好友服务处理器
#[derive(Clone)]
pub struct FriendServiceHandler;

impl FriendServiceHandler {
    /// 处理好友服务请求
    pub async fn handle_request(
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<UserInfo>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理好友服务请求: {} {}", method, path);

        // 获取好友服务客户端
        let mut client = common::service::friend_client().await?;

        // 从JWT中获取用户ID
        let user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;

        // 从路径提取方法名 - 格式: /api/friends/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 发送好友请求
            (&Method::POST, "sendRequest") => {
                let message = extract_string_param(&body, "message", Some("message"))?;
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;
                
                // 校验是否尝试添加自己为好友
                if friend_id == user_id {
                    return Ok(error_response("不能添加自己为好友", StatusCode::BAD_REQUEST));
                }
                
                let request = proto::friend::SendFriendRequestRequest {
                    user_id: user_id.clone(),
                    friend_id: friend_id.clone(),
                    message: message.clone(),
                };

                let response = client.send_friend_request(request).await?;
                let friendship = response.into_inner().friendship.ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;

                Ok(success_response(Self::convert_friendship_to_json(&friendship, &user_id), StatusCode::OK))
            }

            // 接受好友请求
            (&Method::POST, "acceptRequest") => {
                let request_id = extract_string_param(&body, "requestId", Some("request_id"))?;

                let request = proto::friend::AcceptFriendRequestRequest {
                    user_id: user_id.clone(),
                    request_id: request_id.clone(),
                };

                let response = client.accept_friend_request(request).await?;
                let friendship = response.into_inner().friendship.ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;

                Ok(success_response(Self::convert_friendship_to_json(&friendship, &user_id), StatusCode::OK))
            }

            // 拒绝好友请求
            (&Method::POST, "rejectRequest") => {
                let reason = extract_string_param(&body, "rejectReason", Some("reject_reason"))?;
                let request_id = get_optional_string(&body, "requestId", Some("request_id")).unwrap_or_default();

                let request = proto::friend::RejectFriendRequestRequest {
                    user_id: user_id.clone(),
                    reason: reason.clone(),
                    request_id: request_id.clone(),
                };

                let response = client.reject_friend_request(request).await?;
                let friendship = response.into_inner().friendship.ok_or_else(|| anyhow::anyhow!("好友关系数据为空"))?;

                Ok(success_response(Self::convert_friendship_to_json(&friendship, &user_id), StatusCode::OK))
            }

            // 获取好友列表 (废弃)
            (&Method::POST, "getList") => {
                // 提取分页和排序参数
                let page = get_i64_param(&body, "page", 1);
                let page_size = get_i64_param(&body, "pageSize", 20);
                let sort_by = body.get("sortBy").and_then(|v| v.as_str()).unwrap_or("");
                // 提取搜索关键词
                let keyword = get_optional_string(&body, "keyword", None).unwrap_or_default();

                let request = proto::friend::GetFriendListRequest {
                    user_id: user_id.clone(),
                    page,
                    page_size,
                    sort_by: sort_by.to_string(),
                    keyword: keyword.clone(),
                };

                let response = client.get_friend_list(request).await?;
                let inner = response.into_inner();

                let friends = inner.friends.iter().map(|f| Self::convert_friend_to_json(f)).collect::<Vec<_>>();

                Ok(success_response(json!({
                    "friends": friends,
                    "total": inner.total
                }), StatusCode::OK))
            }

            // 获取好友请求列表
            (&Method::POST, "getRequests") => {
                let page = get_i64_param(&body, "page", 1);
                let page_size = get_i64_param(&body, "pageSize", 20);
                
                let request = proto::friend::GetFriendRequestsRequest {
                    user_id: user_id.clone(),
                    page,
                    page_size,
                };

                let response = client.get_friend_requests(request).await?;
                let inner = response.into_inner();
                
                let requests = inner.requests.iter()
                    .map(|r| Self::convert_friendship_to_json(r, &user_id))
                    .collect::<Vec<_>>();

                Ok(success_response(json!({
                    "requests": requests,
                    "total": inner.total
                }), StatusCode::OK))
            }

            // 获取好友列表（包含详细信息）
            (&Method::POST, "getDetailList") | (&Method::GET, "getAllFriends") => {
                // 调用无分页好友详细列表接口
                let request = proto::friend::GetAllFriendDetailListRequest {
                    user_id: user_id.clone(),
                };

                let response = client.get_all_friend_detail_list(request).await?;
                let inner = response.into_inner();

                // 将好友数据转换为JSON
                let detailed_friends = inner.friends.iter()
                    .map(|f| Self::convert_detailed_friend_to_json(f))
                    .collect::<Vec<_>>();

                Ok(success_response(json!({"friends": detailed_friends}), StatusCode::OK))
            }

            // 删除好友
            (&Method::POST, "delete") => {
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;

                let request = proto::friend::DeleteFriendRequest {
                    user_id: user_id.clone(),
                    friend_id: friend_id.clone(),
                };

                let response = client.delete_friend(request).await?;
                let inner = response.into_inner();

                Ok(success_response(inner.success, StatusCode::OK))
            }

            // 检查好友关系
            (&Method::GET, "checkFriendship") => {
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;

                let request = proto::friend::CheckFriendshipRequest {
                    user_id: user_id.clone(),
                    friend_id: friend_id.clone(),
                };

                let response = client.check_friendship(request).await?;
                let inner = response.into_inner();

                let status_text = match inner.status {
                    0 => "PENDING",
                    1 => "ACCEPTED",
                    2 => "REJECTED",
                    3 => "BLOCKED",
                    4 => "EXPIRED",
                    _ => "UNKNOWN"
                };

                Ok(success_response(
                    json!({
                        "status": inner.status,
                        "statusText": status_text
                    }),
                    StatusCode::OK
                ))
            }

            // 拉黑用户
            (&Method::POST, "block") => {
                let blocked_user_id = extract_string_param(&body, "blockedUserId", Some("blocked_user_id"))?;

                let request = proto::friend::BlockUserRequest {
                    user_id: user_id.clone(),
                    blocked_user_id: blocked_user_id.clone(),
                };

                let response = client.block_user(request).await?;
                let inner = response.into_inner();

                Ok(success_response(inner.success, StatusCode::OK))
            }

            // 解除拉黑
            (&Method::POST, "unblock") => {
                let blocked_user_id = extract_string_param(&body, "blockedUserId", Some("blocked_user_id"))?;

                let request = proto::friend::UnblockUserRequest {
                    user_id: user_id.clone(),
                    blocked_user_id: blocked_user_id.clone(),
                };

                let response = client.unblock_user(request).await?;
                let inner = response.into_inner();

                Ok(success_response(inner.success, StatusCode::OK))
            }

            // 创建或更新好友分组
            (&Method::POST, "createOrUpdateGroup") => {
                let id = get_optional_string(&body, "id",None);
                let group_name = extract_string_param(&body, "groupName", Some("group_name"))?;
                let sort_order = body.get("sortOrder").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let friend_ids = body.get("friendIds")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow::anyhow!("friendIds 必须是数组"))?
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<String>>();

                let request = proto::friend::CreateOrUpdateFriendGroupRequest {
                    id,
                    user_id: user_id.clone(),
                    group_name: group_name.clone(),
                    sort_order,
                    friend_ids: friend_ids.clone(),
                };

                let response = client.create_or_update_friend_group(request).await?;
                let inner = response.into_inner();

                let group = inner.group.ok_or_else(|| anyhow::anyhow!("分组数据为空"))?;
                let result = json!({
                    "group": Self::convert_friend_group_to_json(&group),
                    "friendIds": inner.friend_ids
                });

                Ok(success_response(result, StatusCode::OK))
            }

            // 删除好友分组
            (&Method::POST, "deleteGroup") => {
                let id = extract_string_param(&body, "id", Some("id"))?;

                let request = proto::friend::DeleteFriendGroupRequest {
                    id: id.clone(),
                    user_id: user_id.clone(),
                };

                let response = client.delete_friend_group(request).await?;
                let inner = response.into_inner();

                Ok(success_response(inner.success, StatusCode::OK))
            }

            // 获取好友分组列表
            (&Method::GET, "getGroups") => {
                let request = proto::friend::GetFriendGroupsRequest {
                    user_id: user_id.clone(),
                };

                let response = client.get_friend_groups(request).await?;
                let inner = response.into_inner();

                let groups = inner.groups.iter().map(|g| Self::convert_friend_group_to_json(g)).collect::<Vec<_>>();

                Ok(success_response(json!({"groups": groups}), StatusCode::OK))
            }

            // 获取分组好友列表
            (&Method::POST, "getGroupFriends") => {
                let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;

                let request = proto::friend::GetGroupFriendsRequest {
                    group_id: group_id.clone(),
                    user_id: user_id.clone(),
                };

                let response = client.get_group_friends(request).await?;
                let inner = response.into_inner();

                let friends = inner.friends.iter().map(|f| Self::convert_friend_to_json(f)).collect::<Vec<_>>();

                Ok(success_response(json!({
                    "friends": friends,
                    "total": inner.total
                }), StatusCode::OK))
            }
            
            // 搜索潜在好友（通过custom_id或手机号精确搜索）
            (&Method::POST, "searchFriends") => {
                let search_term = extract_string_param(&body, "searchTerm", Some("search_term"))?;
                
                let request = proto::friend::SearchPotentialFriendsRequest {
                    user_id: user_id.clone(),
                    search_term: search_term.clone(),
                };

                // 调用搜索接口，如果出错则返回空列表
                let response = match client.search_potential_friends(request).await {
                    Ok(response) => response,
                    Err(e) => {
                        error!("搜索潜在好友失败: {}", e);
                        // 返回空数组，而不是错误
                        return Ok(success_response(json!([]), StatusCode::OK));
                    }
                };
                
                let inner = response.into_inner();
                let users = inner.users.iter().map(|u| Self::convert_potential_friend_to_json(u)).collect::<Vec<_>>();
                
                // 直接返回用户数组，不用对象包裹
                Ok(success_response(json!({"users": users}), StatusCode::OK))
            }

            // 设置好友星标状态
            (&Method::POST, "toggleStar") => {
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;
                let is_starred = body.get("isStarred").and_then(|v| v.as_bool()).unwrap_or(true);

                let request = proto::friend::ToggleFriendStarRequest {
                    user_id: user_id.clone(),
                    friend_id: friend_id.clone(),
                    is_starred,
                };

                let response = client.toggle_friend_star(request).await?;
                let inner = response.into_inner();

                Ok(success_response(inner.success, StatusCode::OK))
            }

            // 设置好友置顶状态
            (&Method::POST, "toggleTop") => {
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;
                let is_top = body.get("isTop").and_then(|v| v.as_bool()).unwrap_or(true);

                let request = proto::friend::ToggleFriendTopRequest {
                    user_id: user_id.clone(),
                    friend_id: friend_id.clone(),
                    is_top,
                };

                let response = client.toggle_friend_top(request).await?;
                let inner = response.into_inner();

                Ok(success_response(inner.success, StatusCode::OK))
            }

            // 更新好友备注
            (&Method::POST, "updateRemark") => {
                let friend_id = extract_string_param(&body, "friendId", Some("friend_id"))?;
                let remark = extract_string_param(&body, "remark", Some("remark"))?;
                
                let request = proto::friend::UpdateFriendRemarkRequest {
                    user_id: user_id.clone(),
                    friend_id: friend_id.clone(),
                    remark: remark.clone(),
                };

                let response = client.update_friend_remark(request).await?;
                let inner = response.into_inner();
                
                // 如果有好友详细信息，转换并返回
                if let Some(friend) = inner.friend {
                    Ok(success_response(
                        json!({"success": inner.success,
                            "friend":Self::convert_detailed_friend_to_json(&friend)}),
                        StatusCode::OK
                    ))
                } else {
                    Ok(success_response(inner.success, StatusCode::OK))
                }
            }

            // 其他未实现的方法
            _ => {
                error!("好友服务不支持的方法: {} {}", method, method_name);
                Err(anyhow::anyhow!("好友服务不支持的方法: {}", method_name))
            }
        }
    }

    /// 将好友关系消息转换为JSON
    fn convert_friendship_to_json(friendship: &proto::friend::Friendship, current_user_id: &str) -> Value {
        let status_text = match friendship.status {
            0 => "PENDING",
            1 => "ACCEPTED",
            2 => "REJECTED",
            3 => "BLOCKED",
            4 => "EXPIRED",
            _ => "UNKNOWN"
        };

        json!({
            "requestId": friendship.id,
            "userId": friendship.user_id,
            "friendId": friendship.friend_id,
            "status": friendship.status,
            "statusText": status_text,
            "createdAt": timestamp_to_datetime_string(&friendship.created_at),
            "updatedAt": timestamp_to_datetime_string(&friendship.updated_at),
            "message": friendship.message,
            "rejectReason": friendship.reject_reason,
            "friendUsername": friendship.friend_username,
            "friendNickname": friendship.friend_nickname,
            "friendAvatarUrl": friendship.friend_avatar_url,
            "isSelf": friendship.user_id == current_user_id,
        })
    }

    /// 将好友消息转换为JSON
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

    /// 将好友分组消息转换为JSON
    fn convert_friend_group_to_json(group: &proto::friend::FriendGroup) -> Value {
        json!({
            "id": group.id,
            "userId": group.user_id,
            "groupName": group.group_name,
            "sortOrder": group.sort_order,
            "createdAt": timestamp_to_datetime_string(&group.created_at),
            "updatedAt": timestamp_to_datetime_string(&group.updated_at),
            "friendCount": group.friend_count,
        })
    }
    
    /// 将潜在好友消息转换为JSON
    fn convert_potential_friend_to_json(friend: &proto::friend::PotentialFriend) -> Value {
        let status_text = match friend.friendship_status {
            -1 => "NONE",      // 无关系
            1 => "ACCEPTED",   // 已接受/已是好友
            2 => "REJECTED",   // 已拒绝
            _ => "UNKNOWN"     // 未知状态
        };
        
        json!({
            "id": friend.id,
            "username": friend.username,
            "nickname": friend.nickname,
            "avatarUrl": friend.avatar_url,
            "phone": friend.phone,
            "friendshipStatus": friend.friendship_status,
            "friendshipStatusText": status_text
        })
    }

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
} 