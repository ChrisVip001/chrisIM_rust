use async_trait::async_trait;
use bytes::Bytes;
use common::config::AppConfig;
use common::error::Error;
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha1::Sha1;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use chrono::Utc;
use crate::{Oss};

/// 阿里云OSS客户端实现
#[derive(Debug, Clone)]
pub struct AliClient {
    endpoint: String,
    access_key_id: String,
    access_key_secret: String,
    bucket_name: String,
    avatar_bucket_name: String,
    http_client: Client,
}

impl AliClient {
    /// 构造函数，初始化阿里云OSS客户端
    pub fn new(config: &AppConfig) -> Self {
        let oss_config = &config.oss;

        Self {
            endpoint: oss_config.endpoint.clone(),
            access_key_id: oss_config.access_key.clone(),
            access_key_secret: oss_config.secret_key.clone(),
            bucket_name: oss_config.bucket.clone(),
            avatar_bucket_name: oss_config.avatar_bucket.clone(),
            http_client: Client::new(),
        }
    }

    /// 生成阿里云OSS签名
    fn generate_signature(&self, method: &str, key: &str, content_type: &str, expires: i64) -> String {
        let string_to_sign = format!(
            "{}\n\n{}\n{}\n/{}/{}",
            method, content_type, expires, self.bucket_name, key
        );

        let mut mac =
            Hmac::<Sha1>::new_from_slice(self.access_key_secret.as_bytes()).expect("无效的密钥");
        mac.update(string_to_sign.as_bytes());
        let result = mac.finalize();
        base64::encode(result.into_bytes())
    }
}

#[async_trait]
impl Oss for AliClient {
    /// 检查文件是否存在
    async fn file_exists(&self, key: &str, local_md5: &str) -> Result<bool, Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.bucket_name, key);
        let response = match self.http_client.head(&url).send().await{
            Ok(resp) => resp,
            Err(err) => return Err(Error::from(err.to_string())),
        };

        if response.status().is_success() {
            let remote_md5 = response.headers().get("ETag").and_then(|etag| etag.to_str().ok());
            Ok(remote_md5 == Some(local_md5))
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "检查文件是否存在失败: {}",
                err_text
            )))
        }
    }

    /// 上传文件
    async fn upload_file(&self, key: &str, content: Vec<u8>) -> Result<(), Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.bucket_name, key);
        let expires = Utc::now().timestamp() + 3600; // 签名有效期1小时
        let signature = self.generate_signature("PUT", key, "application/octet-stream", expires);

        let response = match self.http_client.put(&url)
            .header("Authorization", format!("OSS {}:{}", self.access_key_id, signature))
            .body(content).send().await{
                Ok(resp) => resp,
                Err(err) => return Err(Error::from(err.to_string())),
            };

        if response.status().is_success() {
            Ok(())
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "上传文件失败: {}",
                err_text
            )))
        }
    }

    /// 下载文件
    async fn download_file(&self, key: &str) -> Result<Bytes, Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.bucket_name, key);
        let response = match self.http_client.get(&url).send().await{
            Ok(resp) => resp,
            Err(err) => return Err(Error::from(err.to_string())),
        };

        if response.status().is_success() {
            Ok(response.bytes().await.map_err(|e| Error::from(format!("获取字节失败: {}", e)))?)
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "下载文件失败: {}",
                err_text
            )))
        }
    }

    /// 删除文件
    async fn delete_file(&self, key: &str) -> Result<(), Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.bucket_name, key);
        let expires = Utc::now().timestamp() + 3600; // 签名有效期1小时
        let signature = self.generate_signature("DELETE", key, "", expires);

        let response = match self.http_client.delete(&url)
            .header("Authorization", format!("OSS {}:{}", self.access_key_id, signature))
            .send().await{
                Ok(resp) => resp,
                Err(err) => return Err(Error::from(err.to_string())),
            };

        if response.status().is_success() {
            Ok(())
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "删除文件失败: {}",
                err_text
            )))
        }
    }

    /// 上传头像
    async fn upload_avatar(&self, key: &str, content: Vec<u8>) -> Result<(), Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.avatar_bucket_name, key);
        let expires = Utc::now().timestamp() + 3600; // 签名有效期1小时
        let signature = self.generate_signature("PUT", key, "application/octet-stream", expires);

        let response = match self.http_client.put(&url)
            .header("Authorization", format!("OSS {}:{}", self.access_key_id, signature))
            .body(content).send()
            .await{
                Ok(resp) => resp,
                Err(err) => return Err(Error::from(err.to_string())),
            };

        if response.status().is_success() {
            Ok(())
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "上传头像失败: {}",
                err_text
            )))
        }
    }

    /// 下载头像
    async fn download_avatar(&self, key: &str) -> Result<Bytes, Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.avatar_bucket_name, key);
        let response = match self.http_client.get(&url).send().await{
            Ok(resp) => resp,
            Err(err) => return Err(Error::from(err.to_string())),
        };

        if response.status().is_success() {
            Ok(response.bytes().await.map_err(|e| Error::from(format!("获取字节失败: {}", e)))?)
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "下载头像失败: {}",
                err_text
            )))
        }
    }

    /// 删除头像
    async fn delete_avatar(&self, key: &str) -> Result<(), Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.avatar_bucket_name, key);
        let expires = Utc::now().timestamp() + 3600; // 签名有效期1小时
        let signature = self.generate_signature("DELETE", key, "", expires);

        let response = match self.http_client.delete(&url)
            .header("Authorization", format!("OSS {}:{}", self.access_key_id, signature))
            .send().await{
                Ok(resp) => resp,
                Err(err) => return Err(Error::from(err.to_string())),
            };

        if response.status().is_success() {
            Ok(())
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "删除头像失败: {}",
                err_text
            )))
        }
    }

    /// 生成预签名URL用于前端直接上传
    async fn generate_presigned_upload_url(
        &self,
        key: &str,
        content_type: &str,
        expiration: Duration,
    ) -> Result<String, Error> {
        let expires = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap() + expiration).as_secs();
        let signature = self.generate_signature("PUT", key, content_type, expires as i64);

        let url = format!(
            "{}/{}/{}?Expires={}&OSSAccessKeyId={}&Signature={}",
            self.endpoint, self.bucket_name, key, expires, self.access_key_id, signature
        );

        Ok(url)
    }

    /// 验证上传是否完成并处理后续逻辑
    async fn validate_upload(&self, key: &str, expected_size: usize, expected_md5: &str) -> Result<bool, Error> {
        let url = format!("{}/{}/{}", self.endpoint, self.bucket_name, key);
        let response = match self.http_client.head(&url).send().await{
            Ok(resp) => resp,
            Err(err) => return Err(Error::from(err.to_string())),
        };

        if response.status().is_success() {
            let remote_size = response.content_length().unwrap_or(0);
            let remote_md5 = response.headers().get("ETag").and_then(|etag| etag.to_str().ok());

            Ok(remote_size == expected_size as u64 && remote_md5 == Some(expected_md5))
        } else {
            let err_text = match response.text().await {
                Ok(t) => t,
                Err(e) => return Err(Error::from(format!("获取响应文本错误: {}", e))),
            };
            Err(Error::from(format!(
                "Failed to validate upload: {}",
                err_text
            )))
        }
    }
}
