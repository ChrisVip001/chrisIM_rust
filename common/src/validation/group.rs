use crate::grpc_client::GroupServiceGrpcClient;
use crate::proto::group::{GetGroupRequest, GetGroupMemberRequest, MemberRole};
use crate::validation::{ValidationResult, UserValidator};
use tonic::Status;
use tracing::{error, info};

// 使用宏导入
use crate::generate_grpc_client;

// 自动生成group-service客户端
generate_grpc_client!(
    name: InternalGroupClient, 
    service: "group-service",
    proto_path: crate::proto::group,
    client_type: group_service_client::GroupServiceClient,
    methods: [
        get_group(GetGroupRequest) -> GetGroupResponse,
        get_group_member(GetGroupMemberRequest) -> GetGroupMemberResponse
    ]
);

/// 群组验证器
pub struct GroupValidator {
    client: GroupServiceGrpcClient,
    user_validator: UserValidator,
}

impl GroupValidator {
    /// 创建新的群组验证器
    pub fn new() -> Self {
        Self {
            client: GroupServiceGrpcClient::from_env(),
            user_validator: UserValidator::new(),
        }
    }
    
    /// 使用已有的客户端创建
    pub fn with_client(client: GroupServiceGrpcClient) -> Self {
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
    
    /// 验证群组是否存在
    pub async fn validate_group_exists(&self, group_id: &str) -> ValidationResult<()> {
        match self.client.get_group(GetGroupRequest {
            group_id: group_id.to_string(),
        }).await {
            Ok(response) => {
                if response.group.is_none() {
                    return Err(Status::not_found(format!("群组 {} 不存在", group_id)));
                }
                Ok(())
            }
            Err(e) => {
                error!("获取群组信息失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
    
    /// 验证用户是否为群组成员
    pub async fn validate_is_member(&self, user_id: &str, group_id: &str) -> ValidationResult<()> {
        match self.client.get_group_member(GetGroupMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                if response.member.is_none() {
                    return Err(Status::permission_denied(format!(
                        "用户 {} 不是群组 {} 的成员", 
                        user_id, group_id
                    )));
                }
                Ok(())
            }
            Err(e) => {
                error!("获取群组成员信息失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
    
    /// 验证用户是否为群组管理员
    pub async fn validate_is_admin(&self, user_id: &str, group_id: &str) -> ValidationResult<()> {
        match self.client.get_group_member(GetGroupMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                match response.member {
                    Some(member) => {
                        if member.role != MemberRole::Admin as i32 && member.role != MemberRole::Owner as i32 {
                            return Err(Status::permission_denied("需要管理员权限"));
                        }
                        Ok(())
                    }
                    None => {
                        Err(Status::permission_denied(format!(
                            "用户 {} 不是群组 {} 的成员", 
                            user_id, group_id
                        )))
                    }
                }
            }
            Err(e) => {
                error!("获取群组成员信息失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
    
    /// 验证用户是否为群主
    pub async fn validate_is_owner(&self, user_id: &str, group_id: &str) -> ValidationResult<()> {
        match self.client.get_group_member(GetGroupMemberRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                match response.member {
                    Some(member) => {
                        if member.role != MemberRole::Owner as i32 {
                            return Err(Status::permission_denied("需要群主权限"));
                        }
                        Ok(())
                    }
                    None => {
                        Err(Status::permission_denied(format!(
                            "用户 {} 不是群组 {} 的成员", 
                            user_id, group_id
                        )))
                    }
                }
            }
            Err(e) => {
                error!("获取群组成员信息失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
}

// 实现通用Validator特征
impl crate::validation::Validator for GroupValidator {
    fn new() -> Self {
        Self::new()
    }
    
    async fn can_perform(&self, operation: &str, subject_id: &str, object_id: Option<&str>) -> ValidationResult<()> {
        // 首先验证用户状态
        self.user_validator.validate_user_status(subject_id).await?;
        
        // 确保有操作对象ID
        let target_id = match object_id {
            Some(id) => id,
            None => return Err(Status::invalid_argument("缺少操作对象ID"))
        };
        
        // 验证群组存在
        self.validate_group_exists(target_id).await?;
        
        // 根据操作类型验证权限
        match operation {
            "join_group" => {
                // 加入群组只需要验证用户状态和群组存在，已经完成
                Ok(())
            }
            "leave_group" => {
                // 验证是否为群成员
                self.validate_is_member(subject_id, target_id).await
            }
            "kick_member" | "invite_member" | "update_group" => {
                // 需要管理员权限
                self.validate_is_admin(subject_id, target_id).await
            }
            "dissolve_group" | "transfer_ownership" => {
                // 需要群主权限
                self.validate_is_owner(subject_id, target_id).await
            }
            "send_group_message" => {
                // 发送消息需要是群成员
                self.validate_is_member(subject_id, target_id).await
            }
            _ => {
                info!("未知的群组操作类型: {}", operation);
                Err(Status::unimplemented(format!("不支持的操作: {}", operation)))
            }
        }
    }
} 