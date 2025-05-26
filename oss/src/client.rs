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

use crate::{calculate_md5, default_avatars, Oss};

#[derive(Debug, Clone)]
pub(crate) struct S3Client {
    bucket: String,
    avatar_bucket: String,
    client: Client,
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

        let config = Builder::new()
            .region(Region::new(config.oss.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(&config.oss.endpoint)
            // use latest behavior version, have to set it manually,
            // although we turn on the feature
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .build();

        let client = Client::from_conf(config);

        let self_ = Self {
            client,
            bucket,
            avatar_bucket,
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
}
