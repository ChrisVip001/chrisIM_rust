use async_trait::async_trait;
use crate::Result;
use super::VerificationAction;

/// 短信验证码服务接口
#[async_trait]
pub trait SmsService: Send + Sync {
    /// 发送短信验证码
    /// 
    /// # 参数
    /// * `phone` - 手机号码(注意要带国家代码，如+86)
    /// * `action` - 验证码用途，如注册、登录等
    /// 
    /// # 返回
    /// * `Result<String>` - 成功返回验证码，失败返回错误
    async fn send_verification_code(&self, phone: &str, action: VerificationAction) -> Result<String>;
    
    /// 验证短信验证码
    /// 
    /// # 参数
    /// * `phone` - 手机号码(注意要带国家代码，如+86)
    /// * `code` - 用户输入的验证码
    /// * `action` - 验证码用途，如注册、登录等
    /// 
    /// # 返回
    /// * `Result<bool>` - 验证成功返回true，失败返回错误
    async fn verify_code(&self, phone: &str, code: &str, action: VerificationAction) -> Result<bool>;
} 