use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bytes::Bytes;
use common::config::AppConfig;
use common::error::Error;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use reqwest::{
    header::HeaderMap, header::HeaderName, header::HeaderValue, Client, StatusCode,
};
use serde_json::json;
use sha1::Sha1;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tracing::{error, info};

use crate::{calculate_md5, default_avatars, Oss, UploadSignature, BucketType};

type HmacSha1 = Hmac<Sha1>;

// 阿里云OSS客户端实现
#[derive(Debug, Clone)]
pub struct OssClient {
    region: String,
    access_key_id: String,
    access_key_secret: String,
    bucket: String,
    avatar_bucket: String,
    domain: String,
    sts_token: Option<String>,
    http_client: Client,
}

impl OssClient {
    pub async fn new(config: &AppConfig) -> Self {
        let region = config.oss.region.clone();
        let access_key_id = config.oss.access_key.clone();
        let access_key_secret = config.oss.secret_key.clone();
        let bucket = config.oss.bucket.clone();
        let avatar_bucket = config.oss.avatar_bucket.clone();
        let domain = config.oss.oss_domain.clone().unwrap_or_default();
        let sts_token = config.oss.oss_sts_token.clone();

        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        let self_ = Self {
            region,
            access_key_id,
            access_key_secret,
            bucket,
            avatar_bucket,
            domain,
            sts_token,
            http_client,
        };

        self_.check_default_avatars().await.unwrap_or_else(|e| {
            error!("Failed to check default avatars: {}", e);
        });

        self_
    }

    // 检查默认头像是否存在，不存在则上传
    async fn check_default_avatars(&self) -> Result<(), Error> {
        for (path, name) in default_avatars().into_iter() {
            if !self.exists_by_name(&self.avatar_bucket, &name).await {
                if let Ok(data) = fs::read(&path).await {
                    self.upload_avatar(&name, data).await?;
                }
            }
        }
        Ok(())
    }

    // 构建阿里云OSS URL
    fn build_oss_url(&self, bucket: &str, key: &str) -> String {
        if !self.domain.is_empty() {
            format!("{}/{}", self.domain.trim_end_matches('/'), key)
        } else {
            format!("https://{}.{}.aliyuncs.com/{}", bucket, self.region, key)
        }
    }

    // 生成OSS API需要的Authorization签名
    async fn generate_auth_string(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        content_md5: &str,
        date: &str,
    ) -> Result<String, Error> {
        // 构建要签名的字符串
        let string_to_sign = format!(
            "{}\n{}\n{}\n{}\n{}",
            method, content_md5, content_type, date, path
        );

        // 计算HMAC-SHA1签名
        let mut mac = HmacSha1::new_from_slice(self.access_key_secret.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(string_to_sign.as_bytes());
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());

        Ok(format!("OSS {}:{}", self.access_key_id, signature))
    }

    // 检查对象是否存在
    async fn exists_by_name(&self, bucket: &str, key: &str) -> bool {
        let url = self.build_oss_url(bucket, key);
        match self.http_client.head(&url).send().await {
            Ok(response) => response.status() == StatusCode::OK,
            Err(_) => false,
        }
    }

    // 上传文件到OSS
    async fn upload(&self, bucket: &str, key: &str, content: Vec<u8>) -> Result<(), Error> {
        let url = self.build_oss_url(bucket, key);
        let content_md5 = calculate_md5(&content);
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();

        let auth = self
            .generate_auth_string("PUT", &format!("/{}/{}", bucket, key), "application/octet-stream", &content_md5, &date)
            .await?;

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_str(&auth).map_err(|e| Error::Internal(format!("Invalid header value: {}", e)))?);
        headers.insert("Date", HeaderValue::from_str(&date).map_err(|e| Error::Internal(format!("Invalid header value: {}", e)))?);
        headers.insert("Content-Type", HeaderValue::from_static("application/octet-stream"));
        headers.insert("Content-MD5", HeaderValue::from_str(&content_md5).map_err(|e| Error::Internal(format!("Invalid header value: {}", e)))?);

        let response = self
            .http_client
            .put(&url)
            .headers(headers)
            .body(content)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to upload file: {}", e)))?;

        if response.status().is_success() {
            info!("Successfully uploaded file to OSS: bucket={}, key={}", bucket, key);
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| "Unable to read response body".to_string());
            error!("Failed to upload file to OSS: status={}, body={}", status, body);
            Err(Error::Internal(format!("Failed to upload file to OSS: {}", status)))
        }
    }

    // 从OSS下载文件
    async fn download(&self, bucket: &str, key: &str) -> Result<Bytes, Error> {
        let url = self.build_oss_url(bucket, key);
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();

        let auth = self
            .generate_auth_string("GET", &format!("/{}/{}", bucket, key), "", "", &date)
            .await?;

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", auth)
            .header("Date", date)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to download file: {}", e)))?;

        if response.status().is_success() {
            let bytes = response
                .bytes()
                .await
                .map_err(|e| Error::Internal(format!("Failed to read response body: {}", e)))?;

            info!("Successfully downloaded file from OSS: bucket={}, key={}, size={}", bucket, key, bytes.len());
            Ok(bytes)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| "Unable to read response body".to_string());
            error!("Failed to download file from OSS: status={}, body={}", status, body);
            Err(Error::Internal(format!("Failed to download file from OSS: {}", status)))
        }
    }

    // 从OSS删除文件
    async fn delete(&self, bucket: &str, key: &str) -> Result<(), Error> {
        let url = self.build_oss_url(bucket, key);
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();

        let auth = self
            .generate_auth_string("DELETE", &format!("/{}/{}", bucket, key), "", "", &date)
            .await?;

        let response = self
            .http_client
            .delete(&url)
            .header("Authorization", auth)
            .header("Date", date)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to delete file: {}", e)))?;

        if response.status().is_success() {
            info!("Successfully deleted file from OSS: bucket={}, key={}", bucket, key);
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| "Unable to read response body".to_string());
            error!("Failed to delete file from OSS: status={}, body={}", status, body);
            Err(Error::Internal(format!("Failed to delete file from OSS: {}", status)))
        }
    }

    // 获取对象元数据
    async fn get_object_metadata(&self, bucket: &str, key: &str) -> Result<HashMap<String, String>, Error> {
        let url = self.build_oss_url(bucket, key);
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();

        let auth = self
            .generate_auth_string("HEAD", &format!("/{}/{}", bucket, key), "", "", &date)
            .await?;

        let response = self
            .http_client
            .head(&url)
            .header("Authorization", auth)
            .header("Date", date)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to get object metadata: {}", e)))?;

        if response.status().is_success() {
            let headers = response.headers();
            let mut metadata = HashMap::new();

            for (name, value) in headers.iter() {
                if let Ok(value_str) = value.to_str() {
                    metadata.insert(name.as_str().to_string(), value_str.to_string());
                }
            }

            info!("Successfully got object metadata from OSS: bucket={}, key={}", bucket, key);
            Ok(metadata)
        } else {
            let status = response.status();
            error!("Failed to get object metadata from OSS: status={}", status);
            Err(Error::Internal(format!("Failed to get object metadata from OSS: {}", status)))
        }
    }
}

#[async_trait]
impl Oss for OssClient {
    async fn file_exists(&self, key: &str, _local_md5: &str) -> Result<bool, Error> {
        Ok(self.exists_by_name(&self.bucket, key).await)
    }

    async fn upload_file(&self, key: &str, content: Vec<u8>) -> Result<(), Error> {
        self.upload(&self.bucket, key, content).await
    }

    async fn download_file(&self, key: &str) -> Result<Bytes, Error> {
        self.download(&self.bucket, key).await
    }

    async fn delete_file(&self, key: &str) -> Result<(), Error> {
        self.delete(&self.bucket, key).await
    }

    async fn upload_avatar(&self, key: &str, content: Vec<u8>) -> Result<(), Error> {
        self.upload(&self.avatar_bucket, key, content).await
    }

    async fn download_avatar(&self, key: &str) -> Result<Bytes, Error> {
        self.download(&self.avatar_bucket, key).await
    }

    async fn delete_avatar(&self, key: &str) -> Result<(), Error> {
        self.delete(&self.avatar_bucket, key).await
    }

    async fn generate_presigned_upload_url(
        &self,
        key: &str,
        content_type: &str,
        expiration: Duration,
    ) -> Result<String, Error> {
        // 阿里云OSS预签名URL实现 (简化版本)
        // 在生产环境中应该使用阿里云OSS SDK
        let bucket = &self.bucket;
        let expire_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() + expiration.as_secs();

        // 构建签名字符串
        let expires = expire_time.to_string();
        let string_to_sign = format!("GET\n\n{}\n{}\n/{}/{}", content_type, expires, bucket, key);

        // 计算签名
        let mut mac = HmacSha1::new_from_slice(self.access_key_secret.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(string_to_sign.as_bytes());
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());

        // 构建预签名URL
        let url = format!(
            "{}?OSSAccessKeyId={}&Expires={}&Signature={}",
            self.build_oss_url(bucket, key),
            urlencoding::encode(&self.access_key_id),
            expires,
            urlencoding::encode(&signature)
        );

        info!("Generated presigned URL for OSS: bucket={}, key={}", bucket, key);
        Ok(url)
    }

    async fn validate_upload(
        &self,
        key: &str,
        expected_size: usize,
        expected_md5: &str,
    ) -> Result<bool, Error> {
        // 检查文件是否存在
        if !self.exists_by_name(&self.bucket, key).await {
            info!("File does not exist: {}", key);
            return Ok(false);
        }

        // 获取对象元数据
        let metadata = self.get_object_metadata(&self.bucket, key).await?;

        // 验证文件大小
        let content_length = metadata
            .get("content-length")
            .ok_or_else(|| Error::Internal("Content-Length header not found".to_string()))?
            .parse::<usize>()
            .map_err(|e| Error::Internal(format!("Failed to parse Content-Length: {}", e)))?;

        if content_length != expected_size {
            error!("Size mismatch for {}: expected {}, got {}", key, expected_size, content_length);
            return Ok(false);
        }

        // 验证MD5
        if let Some(etag) = metadata.get("etag") {
            let etag_value = etag.trim_matches('"');
            if etag_value != expected_md5 {
                error!("MD5 mismatch for {}: expected {}, got {}", key, expected_md5, etag_value);
                return Ok(false);
            }
        } else {
            // 如果没有ETag，则下载文件并计算MD5
            let content = self.download(&self.bucket, key).await?;
            let actual_md5 = calculate_md5(&content);

            if actual_md5 != expected_md5 {
                error!("MD5 mismatch for {}: expected {}, got {}", key, expected_md5, actual_md5);
                return Ok(false);
            }
        }

        info!("Upload validation successful for {}", key);
        Ok(true)
    }

    async fn generate_upload_signature(
        &self,
        key: &str,
        content_type: &str,
        expiration: Duration,
        bucket_type: BucketType,
    ) -> Result<UploadSignature, Error> {
        let bucket = match bucket_type {
            BucketType::File => &self.bucket,
            BucketType::Avatar => &self.avatar_bucket,
        };

        // 计算过期时间戳
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("Failed to get timestamp: {}", e)))?
            .as_secs();
        let expire_timestamp = now + expiration.as_secs();

        // 构建策略文档 (OSS格式)
        let expiration_time = chrono::DateTime::from_timestamp(expire_timestamp as i64, 0)
            .ok_or_else(|| Error::Internal("Invalid timestamp".to_string()))?
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string();

        let policy = json!({
            "expiration": expiration_time,
            "conditions": [
                {"bucket": bucket},
                ["starts-with", "$key", key.split('/').next().unwrap_or("")],
                ["starts-with", "$Content-Type", content_type.split('/').next().unwrap_or("")],
                ["content-length-range", 0, 100 * 1024 * 1024] // 最大100MB
            ]
        });

        // Base64编码策略
        let policy_b64 = BASE64_STANDARD.encode(policy.to_string());

        // 生成签名 (OSS使用HMAC-SHA1)
        let mut mac = HmacSha1::new_from_slice(self.access_key_secret.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(policy_b64.as_bytes());
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());

        // 构建主机地址
        let host = if !self.domain.is_empty() {
            self.domain.clone()
        } else {
            format!("https://{}.{}.aliyuncs.com", bucket, self.region)
        };

        // OSS可能需要额外的参数 (如STS Token)
        let mut extra = json!({});
        if let Some(ref sts_token) = self.sts_token {
            extra["security-token"] = json!(sts_token);
        }

        Ok(UploadSignature {
            host,
            access_key_id: self.access_key_id.clone(),
            policy: policy_b64,
            signature,
            dir: key.split('/').next().unwrap_or("").to_string(),
            expire: expire_timestamp as i64,
            extra: if extra.as_object().unwrap().is_empty() { None } else { Some(extra) },
        })
    }
} 