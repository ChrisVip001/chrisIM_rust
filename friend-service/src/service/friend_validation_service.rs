use anyhow::Result;
use common::proto::friend::{
    FriendshipStatus, GetFriendListRequest, GetFriendListResponse,
};
use common::proto::user::UserStatus;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info};

use crate::repository::friendship_repository::FriendshipRepository;

// 导入宏
use common::generate_grpc_client;

// 使用宏生成user-service客户端
generate_grpc_client!(
    name: UserServiceGrpcClient, 
    service: "user-service",
    proto_path: common::proto::user,
    client_type: user_service_client::UserServiceClient,
    methods: [
        check_user_status(CheckUserStatusRequest) -> CheckUserStatusResponse,
        get_user_by_id(GetUserByIdRequest) -> UserResponse
    ]
);

// 使用宏生成group-service客户端（如果需要验证用户组关系）
generate_grpc_client!(
    name: GroupServiceGrpcClient, 
    service: "group-service",
    proto_path: common::proto::group,
    client_type: group_service_client::GroupServiceClient,
    methods: [
        get_user_groups(GetUserGroupsRequest) -> GetUserGroupsResponse
    ]
);

/// 扩展的好友验证服务
pub struct FriendValidationService {
    repository: FriendshipRepository,
    user_client: UserServiceGrpcClient,
    group_client: GroupServiceGrpcClient,
}

impl FriendValidationService {
    /// 创建新的验证服务
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: FriendshipRepository::new(pool),
            user_client: UserServiceGrpcClient::from_env(),
            group_client: GroupServiceGrpcClient::from_env(),
        }
    }

    /// 检查用户是否存在且状态正常
    pub async fn validate_user(&self, user_id: &str) -> Result<(), Status> {
        match self.user_client.check_user_status(common::proto::user::CheckUserStatusRequest {
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                if !response.exists {
                    return Err(Status::not_found(format!("用户 {} 不存在", user_id)));
                }
                
                // 根据用户状态返回不同的错误
                match response.status {
                    UserStatus::Active => {
                        // 正常状态，继续处理
                        Ok(())
                    }
                    UserStatus::Banned => {
                        Err(Status::permission_denied(format!("用户 {} 已被禁用", user_id)))
                    }
                    UserStatus::Deleted => {
                        Err(Status::not_found(format!("用户 {} 已被删除", user_id)))
                    }
                    UserStatus::Inactive => {
                        Err(Status::permission_denied(format!("用户 {} 未激活", user_id)))
                    }
                }
            }
            Err(e) => {
                error!("验证用户状态失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }

    /// 验证两个用户是否能够建立好友关系
    pub async fn validate_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Status> {
        // 1. 验证两个用户是否存在且状态正常
        self.validate_user(user_id).await?;
        self.validate_user(friend_id).await?;
        
        // 2. 检查是否为同一用户
        if user_id == friend_id {
            return Err(Status::invalid_argument("不能添加自己为好友"));
        }
        
        // 3. 检查是否已经是好友或有待处理请求
        match self.repository.check_friendship_by_id(user_id, friend_id).await {
            Ok(Some(friendship)) => {
                match friendship.status {
                    FriendshipStatus::Accepted => {
                        return Err(Status::already_exists("已经是好友关系"));
                    }
                    FriendshipStatus::Pending => {
                        return Err(Status::already_exists("已有待处理的好友请求"));
                    }
                    FriendshipStatus::Rejected => {
                        // 可以再次发送请求，但可能需要一些冷却时间限制
                        let rejected_time = friendship.updated_at.unwrap();
                        let now = chrono::Utc::now();
                        
                        // 计算拒绝后的时间差（例如：24小时内不能再次发送）
                        if now.signed_duration_since(rejected_time).num_hours() < 24 {
                            return Err(Status::resource_exhausted(
                                "最近被拒绝，请稍后再试"
                            ));
                        }
                    }
                    FriendshipStatus::Blocked => {
                        return Err(Status::permission_denied("您已被对方屏蔽"));
                    }
                }
            }
            Ok(None) => {
                // 没有现有关系，可以发送请求
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }
        
        // 4. 可以进行额外的业务规则验证
        // 例如：检查用户是否在同一个群组中
        // 这里只是示例，实际可能不需要这个检查
        self.check_common_groups(user_id, friend_id).await?;
        
        Ok(())
    }
    
    /// 检查两个用户是否有共同群组（可选的附加验证）
    async fn check_common_groups(&self, user_id: &str, friend_id: &str) -> Result<(), Status> {
        // 这是一个示例业务规则：只允许同一群组的用户成为好友
        // 实际使用中，这个规则可能不是必须的
        
        // 获取用户的群组
        let user_groups = match self.group_client.get_user_groups(common::proto::group::GetUserGroupsRequest {
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => response.groups,
            Err(e) => {
                // 这里我们不阻止好友添加，只是记录错误
                error!("获取用户群组失败: {}", e);
                return Ok(());
            }
        };
        
        // 获取朋友的群组
        let friend_groups = match self.group_client.get_user_groups(common::proto::group::GetUserGroupsRequest {
            user_id: friend_id.to_string(),
        }).await {
            Ok(response) => response.groups,
            Err(e) => {
                error!("获取朋友群组失败: {}", e);
                return Ok(());
            }
        };
        
        // 检查是否有共同群组
        let user_group_ids: std::collections::HashSet<String> = user_groups
            .iter()
            .map(|g| g.id.clone())
            .collect();
            
        let has_common_group = friend_groups
            .iter()
            .any(|g| user_group_ids.contains(&g.id));
            
        // 这里只是作为示例，实际上可能不需要这个限制
        // 如果需要限制，可以取消下面的注释
        /*
        if !has_common_group && !user_groups.is_empty() && !friend_groups.is_empty() {
            return Err(Status::permission_denied("只能添加同群组的用户为好友"));
        }
        */
        
        info!("用户 {} 和 {} 是否有共同群组: {}", user_id, friend_id, has_common_group);
        Ok(())
    }
    
    /// 获取好友列表并验证每个好友的状态
    pub async fn get_validated_friend_list(
        &self,
        request: Request<GetFriendListRequest>,
    ) -> Result<Response<GetFriendListResponse>, Status> {
        let req = request.into_inner();
        
        // 验证请求用户是否有效
        self.validate_user(&req.user_id).await?;
        
        // 从数据库获取好友列表
        let friends = match self.repository.get_friend_list_by_id(&req.user_id).await {
            Ok(friends) => friends,
            Err(e) => {
                error!("获取好友列表失败: {}", e);
                return Err(Status::internal("获取好友列表失败"));
            }
        };
        
        // 过滤掉状态异常的好友
        let mut valid_friends = Vec::new();
        
        for friend in friends {
            // 检查每个好友的状态
            let status_check = self.user_client.check_user_status(common::proto::user::CheckUserStatusRequest {
                user_id: friend.friend_id.to_string(),
            }).await;
            
            match status_check {
                Ok(status) => {
                    // 只包含存在且状态为ACTIVE的好友
                    if status.exists && status.status == UserStatus::Active as i32 {
                        valid_friends.push(friend.to_proto());
                    } else {
                        info!("好友 {} 状态异常，从列表中过滤", friend.friend_id);
                    }
                }
                Err(e) => {
                    // 如果无法验证状态，记录错误但不中断整个请求
                    error!("验证好友 {} 状态失败: {}", friend.friend_id, e);
                    // 可以选择是否包含无法验证状态的好友
                    valid_friends.push(friend.to_proto());
                }
            }
        }
        
        Ok(Response::new(GetFriendListResponse {
            friends: valid_friends,
        }))
    }
} 