use serde::{Deserialize, Serialize};

/// 腾讯云短信配置
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TencentSmsConfig {
    pub secret_id: String,
    pub secret_key: String,
    pub app_id: String,
    pub sign_name: String,
    pub template_id: String,
    pub expire_seconds: u64,
    pub code_length: u8,
    pub region: String,
    #[serde(default = "default_throttle_enabled")]
    pub throttle_enabled: bool,     // 是否启用防重复发送
    #[serde(default = "default_throttle_seconds")]
    pub throttle_seconds: u64,      // 重复发送限制时间(秒)
}

/// 默认启用防重复发送
fn default_throttle_enabled() -> bool {
    true
}

/// 默认限制60秒内不能重复发送
fn default_throttle_seconds() -> u64 {
    60
}

/// 短信服务配置
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SmsConfig {
    pub tencent: TencentSmsConfig,
} 