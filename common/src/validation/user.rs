use crate::grpc_client::UserServiceGrpcClient;
use crate::proto::user::{CheckUserStatusRequest, UserStatus};
use crate::validation::ValidationResult;
use tonic::Status;
use tracing::{error, info};

// 使用宏导入
use crate::generate_grpc_client;

// 自动生成user-service客户端，如果需要直接在这里使用
generate_grpc_client!(
    name: InternalUserClient, 
    service: "user-service",
    proto_path: crate::proto::user,
    client_type: user_service_client::UserServiceClient,
    methods: [
        check_user_status(CheckUserStatusRequest) -> CheckUserStatusResponse
    ]
);

/// 用户验证器
/// 提供与用户状态和权限相关的验证功能
#[derive(Clone)]
pub struct UserValidator {
    client: UserServiceGrpcClient,
}

impl UserValidator {
    /// 创建新的用户验证器
    pub fn new() -> Self {
        Self {
            client: UserServiceGrpcClient::from_env(),
        }
    }
    
    /// 使用已有的客户端创建
    pub fn with_client(client: UserServiceGrpcClient) -> Self {
        Self { client }
    }
    
    /// 检查用户是否存在且状态正常
    pub async fn validate_user_status(&self, user_id: &str) -> ValidationResult<()> {
        match self.client.check_user_status(CheckUserStatusRequest {
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                if !response.exists {
                    return Err(Status::not_found(format!("用户 {} 不存在", user_id)));
                }
                
                // 根据用户状态返回不同的错误
                match response.status {
                    UserStatus::Active => Ok(()),
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
    
    /// 检查用户ID是否有效 (快速检查)
    pub fn validate_user_id_format(&self, user_id: &str) -> ValidationResult<()> {
        // 检查ID格式是否符合UUID
        if let Err(_) = uuid::Uuid::parse_str(user_id) {
            return Err(Status::invalid_argument(format!("无效的用户ID格式: {}", user_id)));
        }
        Ok(())
    }
    
    /// 检查多个用户
    pub async fn validate_multiple_users(&self, user_ids: &[&str]) -> ValidationResult<()> {
        for user_id in user_ids {
            self.validate_user_status(user_id).await?;
        }
        Ok(())
    }
    
    /// 检查用户是否是自己
    pub fn validate_not_self(&self, user_id: &str, other_id: &str) -> ValidationResult<()> {
        if user_id == other_id {
            return Err(Status::invalid_argument("不能对自己执行此操作"));
        }
        Ok(())
    }
}

// 实现通用Validator特征
impl crate::validation::Validator for UserValidator {
    fn new() -> Self {
        Self::new()
    }
    
    async fn can_perform(&self, operation: &str, subject_id: &str, object_id: Option<&str>) -> ValidationResult<()> {
        // 首先验证主体用户
        self.validate_user_status(subject_id).await?;
        
        // 根据操作类型进行额外验证
        match operation {
            "add_friend" | "accept_friend" | "reject_friend" | "block_user" => {
                if let Some(target_id) = object_id {
                    // 验证目标用户
                    self.validate_user_status(target_id).await?;
                    
                    // 验证不是自己
                    self.validate_not_self(subject_id, target_id)?;
                } else {
                    return Err(Status::invalid_argument("缺少操作对象ID"));
                }
            }
            "join_group" | "leave_group" => {
                // 群组相关操作只需验证用户自己
                // 群组验证会在GroupValidator中进行
            }
            _ => {
                info!("未知的操作类型: {}", operation);
            }
        }
        
        Ok(())
    }
} 