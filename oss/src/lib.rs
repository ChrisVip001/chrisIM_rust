use async_trait::async_trait;
use bytes::Bytes;
use common::config::AppConfig;
use common::error::Error;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use md5::{Digest, Md5};
use serde::Serialize;

mod s3_client;
mod cos_client;
mod oss_client;

// 存储提供商枚举
#[derive(Debug, Clone, PartialEq)]
pub enum StorageProvider {
    S3,     // AWS S3兼容接口(包括MinIO)
    COS,    // 腾讯云对象存储
    OSS,    // 阿里云对象存储
}

// 统一的上传签名结构体
#[derive(Debug, Serialize)]
pub struct UploadSignature {
    pub host: String,
    pub access_key_id: String,
    pub policy: String,
    pub signature: String,
    pub dir: String,
    pub expire: i64,
    pub extra: Option<serde_json::Value>, // 某些平台需要 securityToken 等附加参数
}

// 存储桶类型枚举
#[derive(Debug, Clone, PartialEq)]
pub enum BucketType {
    File,   // 普通文件存储桶
    Avatar, // 头像存储桶
}

#[async_trait]
pub trait Oss: Debug + Send + Sync {
    async fn file_exists(&self, key: &str, local_md5: &str) -> Result<bool, Error>;
    async fn upload_file(&self, key: &str, content: Vec<u8>) -> Result<(), Error>;
    async fn download_file(&self, key: &str) -> Result<Bytes, Error>;
    async fn delete_file(&self, key: &str) -> Result<(), Error>;

    async fn upload_avatar(&self, key: &str, content: Vec<u8>) -> Result<(), Error>;
    async fn download_avatar(&self, key: &str) -> Result<Bytes, Error>;
    async fn delete_avatar(&self, key: &str) -> Result<(), Error>;
    
    // 生成预签名URL用于前端直接上传 (PUT方式)
    async fn generate_presigned_upload_url(&self, key: &str, content_type: &str, expiration: Duration) -> Result<String, Error>;
    
    // 生成POST表单上传签名 (POST方式)
    async fn generate_upload_signature(&self, key: &str, content_type: &str, expiration: Duration, bucket_type: BucketType) -> Result<UploadSignature, Error>;
    
    // 验证上传是否完成并处理后续逻辑
    async fn validate_upload(&self, key: &str, expected_size: usize, expected_md5: &str) -> Result<bool, Error>;
}

pub async fn oss(config: &AppConfig) -> Arc<dyn Oss> {
    // 根据配置选择存储提供商
    match config.oss.provider.as_str() {
        "cos" => Arc::new(cos_client::CosClient::new(config).await),
        "oss" => Arc::new(oss_client::OssClient::new(config).await),
        _ => Arc::new(s3_client::S3Client::new(config).await), // 默认使用S3
    }
}

pub fn default_avatars() -> HashMap<String, String> {
    HashMap::from([
        (
            String::from("./oss/default_avatar/avatar1.png"),
            String::from("avatar1.png"),
        ),
        (
            String::from("./oss/default_avatar/avatar2.png"),
            String::from("avatar2.png"),
        ),
        (
            String::from("./oss/default_avatar/avatar3.png"),
            String::from("avatar3.png"),
        ),
    ])
}

// 计算文件MD5哈希值
fn calculate_md5(content: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(content);
    let result = hasher.finalize();
    format!("{:x}", result)
}
