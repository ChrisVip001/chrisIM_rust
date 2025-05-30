use serde::{Serialize, Deserialize};
use std::str::FromStr;

/// 验证码用途枚举
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationAction {
    #[serde(rename = "register")]
    Register,       // 注册
    #[serde(rename = "login")]
    Login,          // 登录
    #[serde(rename = "reset_password")]
    ResetPassword,  // 重置密码
    #[serde(rename = "bind_phone")]
    BindPhone,      // 绑定手机号
    #[serde(rename = "change_phone")]
    ChangePhone,    // 更换手机号
    #[serde(rename = "deactivate")]
    Deactivate,     // 注销账号
}

impl VerificationAction {
    /// 获取验证码用途的显示文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Register => "注册",
            Self::Login => "登录",
            Self::ResetPassword => "重置密码",
            Self::BindPhone => "绑定手机号",
            Self::ChangePhone => "更换手机号",
            Self::Deactivate => "注销账号",
        }
    }
    
    /// 获取验证码用途的编码
    pub fn as_code(&self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Login => "login",
            Self::ResetPassword => "reset_password",
            Self::BindPhone => "bind_phone",
            Self::ChangePhone => "change_phone",
            Self::Deactivate => "deactivate",
        }
    }
}

impl FromStr for VerificationAction {
    type Err = String;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "register" => Ok(Self::Register),
            "login" => Ok(Self::Login),
            "reset_password" => Ok(Self::ResetPassword),
            "bind_phone" => Ok(Self::BindPhone),
            "change_phone" => Ok(Self::ChangePhone),
            "deactivate" => Ok(Self::Deactivate),
            _ => Err(format!("未知的验证码用途: {}", s)),
        }
    }
}

impl Default for VerificationAction {
    fn default() -> Self {
        Self::Register
    }
} 