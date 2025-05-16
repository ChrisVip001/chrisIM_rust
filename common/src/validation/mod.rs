pub mod user;
pub mod friend;
pub mod group;
pub mod middleware;

// 重新导出常用的验证功能
pub use user::UserValidator;
pub use friend::FriendValidator;
pub use group::GroupValidator;
pub use middleware::ValidationMiddleware;

// 通用验证结果类型
pub type ValidationResult<T> = Result<T, tonic::Status>;

// 通用的验证特质
pub trait Validator {
    // 初始化验证器
    fn new() -> Self;
    
    // 验证是否可以执行指定操作
    async fn can_perform(&self, operation: &str, subject_id: &str, object_id: Option<&str>) -> ValidationResult<()>;
}

// 组合验证器，允许多个验证器一起使用
pub struct CompositeValidator<T: Validator> {
    validators: Vec<T>,
}

impl<T: Validator> CompositeValidator<T> {
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }
    
    pub fn add_validator(&mut self, validator: T) {
        self.validators.push(validator);
    }
    
    pub async fn validate_all(&self, operation: &str, subject_id: &str, object_id: Option<&str>) -> ValidationResult<()> {
        for validator in &self.validators {
            validator.can_perform(operation, subject_id, object_id).await?;
        }
        Ok(())
    }
} 