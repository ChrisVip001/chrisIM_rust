use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OssConfig {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub avatar_bucket: String,
    pub region: String,
    // 存储提供商，可选值: "s3", "cos"，默认为s3
    #[serde(default = "default_provider")]
    pub provider: String,
    // 腾讯云COS专用配置
    #[serde(default)]
    pub cos_app_id: Option<String>,
    #[serde(default)]
    pub cos_domain: Option<String>,
}

// 默认存储提供商为s3
fn default_provider() -> String {
    "s3".to_string()
}