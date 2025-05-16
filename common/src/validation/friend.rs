use crate::grpc_client::FriendServiceGrpcClient;
use crate::proto::friend::{CheckFriendshipRequest, FriendshipStatus};
use crate::validation::{ValidationResult, UserValidator};
use tonic::Status;
use tracing::{error, info};

// 使用宏导入
use crate::generate_grpc_client;

// 自动生成friend-service客户端
generate_grpc_client!(
    name: InternalFriendClient, 
    service: "friend-service",
    proto_path: crate::proto::friend,
    client_type: friend_service_client::FriendServiceClient,
    methods: [
        check_friendship(CheckFriendshipRequest) -> CheckFriendshipResponse
    ]
);

/// 好友关系验证器
/// 提供好友关系相关的验证功能
pub struct FriendValidator {
    client: FriendServiceGrpcClient,
    user_validator: UserValidator,
}

impl FriendValidator {
    /// 创建新的好友验证器
    pub fn new() -> Self {
        Self {
            client: FriendServiceGrpcClient::from_env(),
            user_validator: UserValidator::new(),
        }
    }
    
    /// 使用已有的客户端创建
    pub fn with_client(client: FriendServiceGrpcClient) -> Self {
        Self { 
            client,
            user_validator: UserValidator::new(),
        }
    }
    
    /// 设置用户验证器
    pub fn with_user_validator(mut self, validator: UserValidator) -> Self {
        self.user_validator = validator;
        self
    }
    
    /// 验证两个用户是否能够建立好友关系
    pub async fn validate_can_be_friends(&self, user_id: &str, friend_id: &str) -> ValidationResult<()> {
        // 1. 验证两个用户状态
        self.user_validator.validate_user_status(user_id).await?;
        self.user_validator.validate_user_status(friend_id).await?;
        
        // 2. 验证不是自己
        self.user_validator.validate_not_self(user_id, friend_id)?;
        
        // 3. 检查现有关系
        self.validate_friendship_status(user_id, friend_id).await
    }
    
    /// 检查好友关系状态
    pub async fn validate_friendship_status(&self, user_id: &str, friend_id: &str) -> ValidationResult<()> {
        match self.client.check_friendship(CheckFriendshipRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        }).await {
            Ok(response) => {
                match response.status {
                    FriendshipStatus::Accepted => {
                        return Err(Status::already_exists("已经是好友关系"));
                    }
                    FriendshipStatus::Pending => {
                        return Err(Status::already_exists("已有待处理的好友请求"));
                    }
                    FriendshipStatus::Blocked => {
                        return Err(Status::permission_denied("您已被对方屏蔽"));
                    }
                    FriendshipStatus::Rejected => {
                        // 可以重新发送请求，但可以添加冷却期验证
                        info!("之前的好友请求被拒绝，允许重新发送");
                    }
                }
                Ok(())
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
    
    /// 验证是否已经是好友
    pub async fn validate_are_friends(&self, user_id: &str, friend_id: &str) -> ValidationResult<()> {
        match self.client.check_friendship(CheckFriendshipRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        }).await {
            Ok(response) => {
                if response.status != FriendshipStatus::Accepted {
                    return Err(Status::failed_precondition("不是好友关系"));
                }
                Ok(())
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
    
    /// 验证是否有待处理的好友请求
    pub async fn validate_has_pending_request(&self, user_id: &str, friend_id: &str) -> ValidationResult<()> {
        match self.client.check_friendship(CheckFriendshipRequest {
            user_id: user_id.to_string(),
            friend_id: friend_id.to_string(),
        }).await {
            Ok(response) => {
                if response.status != FriendshipStatus::Pending {
                    return Err(Status::failed_precondition("没有待处理的好友请求"));
                }
                Ok(())
            }
            Err(e) => {
                error!("检查好友请求失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
}

// 实现通用Validator特征
impl crate::validation::Validator for FriendValidator {
    fn new() -> Self {
        Self::new()
    }
    
    async fn can_perform(&self, operation: &str, subject_id: &str, object_id: Option<&str>) -> ValidationResult<()> {
        // 确保有操作对象ID
        let target_id = match object_id {
            Some(id) => id,
            None => return Err(Status::invalid_argument("缺少操作对象ID"))
        };
        
        // 根据操作类型进行验证
        match operation {
            "add_friend" => {
                // 验证能否成为好友
                self.validate_can_be_friends(subject_id, target_id).await
            }
            "accept_friend" | "reject_friend" => {
                // 验证是否有待处理请求
                self.validate_has_pending_request(target_id, subject_id).await
            }
            "remove_friend" => {
                // 验证是否为好友
                self.validate_are_friends(subject_id, target_id).await
            }
            "send_message" => {
                // 验证是否为好友，然后才能发送消息
                self.validate_are_friends(subject_id, target_id).await
            }
            _ => {
                info!("未知的好友操作类型: {}", operation);
                Ok(())
            }
        }
    }
} 