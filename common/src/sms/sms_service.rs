use async_trait::async_trait;
use crate::Result;

/// 短信验证码服务接口
#[async_trait]
pub trait SmsService: Send + Sync {
    /// 发送短信验证码
    /// 
    /// # 参数
    /// * `phone` - 手机号码(注意要带国家代码，如+86)
    /// * `template_param` - 模板参数，如验证码等
    /// 
    /// # 返回
    /// * `Result<String>` - 成功返回验证码，失败返回错误
    async fn send_verification_code(&self, phone: &str) -> Result<String>;
    
    /// 验证短信验证码
    /// 
    /// # 参数
    /// * `phone` - 手机号码(注意要带国家代码，如+86)
    /// * `code` - 用户输入的验证码
    /// 
    /// # 返回
    /// * `Result<bool>` - 验证成功返回true，失败返回错误
    async fn verify_code(&self, phone: &str, code: &str) -> Result<bool>;
} 