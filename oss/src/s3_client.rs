use async_trait::async_trait;
use aws_sdk_s3::config::{Builder, Credentials, Region};
use aws_sdk_s3::operation::get_object::GetObjectOutput;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use aws_smithy_runtime_api::client::result::SdkError;
use bytes::Bytes;
use common::config::AppConfig;
use common::error::Error;
use md5::{Digest, Md5};
use std::time::Duration;
use tokio::fs;
use tracing::{error, info};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::json;
use chrono;

use crate::{calculate_md5, default_avatars, Oss, UploadSignature, BucketType};

type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Clone)]
pub(crate) struct S3Client {
    bucket: String,
    avatar_bucket: String,
    client: Client,
    endpoint: Option<String>,
    region: String,
    access_key_id: String,
    secret_key: String,
}

impl S3Client {
    pub async fn new(config: &AppConfig) -> Self {
        let credentials = Credentials::new(
            &config.oss.access_key,
            &config.oss.secret_key,
            None,
            None,
            "MinioCredentials",
        );

        let bucket = config.oss.bucket.clone();
        let avatar_bucket = config.oss.avatar_bucket.clone();
        let endpoint = Some(config.oss.endpoint.clone());
        let region = config.oss.region.clone();
        let access_key_id = config.oss.access_key.clone();
        let secret_key = config.oss.secret_key.clone();

        let s3_config = Builder::new()
            .region(Region::new(config.oss.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(&config.oss.endpoint)
            // use latest behavior version, have to set it manually,
            // although we turn on the feature
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .build();

        let client = Client::from_conf(s3_config);

        let self_ = Self {
            client,
            bucket,
            avatar_bucket,
            endpoint,
            region,
            access_key_id,
            secret_key,
        };

        self_.create_bucket().await.unwrap();
        self_.check_default_avatars().await.unwrap();
        self_
    }

    async fn check_bucket_exists(&self) -> Result<bool, Error> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_response) => Ok(true),
            Err(SdkError::ServiceError(e)) => {
                if e.raw().status().as_u16() == 404 {
                    Ok(false)
                } else {
                    Err(Error::Internal(
                        "check avatar_bucket exists error".to_string(),
                    ))
                }
            }
            Err(e) => {
                error!("check_bucket_exists error: {:?}", e);
                Err(Error::Internal(e.to_string()))
            }
        }
    }

    async fn check_avatar_bucket_exits(&self) -> Result<bool, Error> {
        match self
            .client
            .head_bucket()
            .bucket(&self.avatar_bucket)
            .send()
            .await
        {
            Ok(_response) => Ok(true),
            Err(SdkError::ServiceError(e)) => {
                if e.raw().status().as_u16() == 404 {
                    Ok(false)
                } else {
                    Err(Error::Internal(
                        "check avatar_bucket exists error".to_string(),
                    ))
                }
            }
            Err(e) => {
                error!("check avatar_bucket exists error: {:?}", e);
                Err(Error::Internal(e.to_string()))
            }
        }
    }

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

    async fn create_bucket(&self) -> Result<(), Error> {
        let is_exist = self.check_bucket_exists().await?;
        if !is_exist {
            self.client
                .create_bucket()
                .bucket(&self.bucket)
                .send()
                .await?;
        }

        if !self.check_avatar_bucket_exits().await? {
            self.client
                .create_bucket()
                .bucket(&self.avatar_bucket)
                .send()
                .await?;
        }
        Ok(())
    }

    // 获取对象的元数据信息
    async fn get_object_metadata(&self, bucket: &str, key: &str) -> Result<GetObjectOutput, Error> {
        let resp = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                error!("Failed to get object metadata: {:?}", e);
                Error::Internal(format!("Failed to get object metadata: {}", e))
            })?;

        Ok(resp)
    }
}

#[async_trait]
impl Oss for S3Client {
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
        // 创建预签名配置
        let presigned_config = PresigningConfig::builder()
            .expires_in(expiration)
            .build()
            .map_err(|e| Error::Internal(format!("Failed to build presigning config: {}", e)))?;

        // 创建预签名上传URL
        let presigned_req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(presigned_config)
            .await
            .map_err(|e| Error::Internal(format!("Failed to generate presigned URL: {}", e)))?;

        info!("Generated presigned URL for {}", key);
        Ok(presigned_req.uri().to_string())
    }

    async fn validate_upload(
        &self,
        key: &str,
        expected_size: usize,
        expected_md5: &str,
    ) -> Result<bool, Error> {
        // 检查文件是否存在
        if !self.exists_by_name(&self.bucket, key).await {
            return Ok(false);
        }

        // 获取对象元数据
        let obj = self.get_object_metadata(&self.bucket, key).await?;

        // 验证文件大小
        let actual_size = match obj.content_length() {
            Some(size) => size as usize,
            None => {
                error!("Failed to get content length for {}", key);
                return Ok(false);
            }
        };

        if actual_size != expected_size {
            error!(
                "Size mismatch for {}: expected {}, got {}",
                key, expected_size, actual_size
            );
            return Ok(false);
        }

        // 下载文件内容并验证MD5
        let content = self.download(&self.bucket, key).await?;
        let actual_md5 = calculate_md5(&content);

        if actual_md5 != expected_md5 {
            error!(
                "MD5 mismatch for {}: expected {}, got {}",
                key, expected_md5, actual_md5
            );
            return Ok(false);
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

        // 生成过期时间字符串 (ISO 8601格式)
        let expiration_date = chrono::DateTime::from_timestamp(expire_timestamp as i64, 0)
            .ok_or_else(|| Error::Internal("Invalid timestamp".to_string()))?
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string();

        // 构建策略文档
        let policy = json!({
            "expiration": expiration_date,
            "conditions": [
                {"bucket": bucket},
                ["starts-with", "$key", key.split('/').next().unwrap_or("")],
                ["starts-with", "$Content-Type", content_type.split('/').next().unwrap_or("")],
                ["content-length-range", 0, 100 * 1024 * 1024] // 最大100MB
            ]
        });

        // Base64编码策略
        let policy_b64 = BASE64_STANDARD.encode(policy.to_string());

        // 使用HMAC-SHA1生成签名 (兼容S3 POST策略签名)
        let signature = self.calculate_signature(&policy_b64)?;

        // 构建主机地址
        let host = if let Some(endpoint) = &self.endpoint {
            endpoint.clone()
        } else {
            format!("https://s3.{}.amazonaws.com", self.region)
        };

        Ok(UploadSignature {
            host,
            access_key_id: self.access_key_id.clone(),
            policy: policy_b64,
            signature,
            dir: key.to_string(),
            expire: expire_timestamp as i64,
            extra: None,
        })
    }
}

impl S3Client {
    async fn exists_by_name(&self, bucket: &str, key: &str) -> bool {
        self.client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .is_ok()
    }

    async fn upload(&self, bucket: &str, key: &str, content: Vec<u8>) -> Result<(), Error> {
        self.client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(content.into())
            .send()
            .await?;
        Ok(())
    }

    async fn download(&self, bucket: &str, key: &str) -> Result<Bytes, Error> {
        let resp = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await?;

        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;

        Ok(data.into_bytes())
    }

    async fn delete(&self, bucket: &str, key: &str) -> Result<(), Error> {
        let client = self.client.clone();
        client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await?;

        Ok(())
    }

    // 计算签名 (S3 POST策略签名使用HMAC-SHA1)
    fn calculate_signature(&self, policy_b64: &str) -> Result<String, Error> {
        // S3 POST表单签名使用HMAC-SHA1算法
        let mut mac = HmacSha1::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(policy_b64.as_bytes());
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());
        
        Ok(signature)
    }
}
