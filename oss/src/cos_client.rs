use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bytes::Bytes;
use common::config::AppConfig;
use common::error::Error;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use reqwest::{
    header::HeaderMap, header::HeaderName, header::HeaderValue, Client, Method, StatusCode,
};
use serde_json::json;
use sha1::Sha1;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tracing::{error, info};
use url::Url;

use crate::{calculate_md5, default_avatars, Oss};

type HmacSha1 = Hmac<Sha1>;

// 腾讯云COS客户端实现
#[derive(Debug, Clone)]
pub struct CosClient {
    region: String,
    app_id: String,
    secret_id: String,
    secret_key: String,
    bucket: String,
    avatar_bucket: String,
    domain: String,
    http_client: Client,
}

impl CosClient {
    pub async fn new(config: &AppConfig) -> Self {
        let region = config.oss.region.clone();
        let app_id = config.oss.cos_app_id.clone().unwrap_or_default();
        let secret_id = config.oss.access_key.clone(); // 复用access_key字段
        let secret_key = config.oss.secret_key.clone(); // 复用secret_key字段
        let bucket = config.oss.bucket.clone();
        let avatar_bucket = config.oss.avatar_bucket.clone();
        let domain = config.oss.cos_domain.clone().unwrap_or_default();

        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        let self_ = Self {
            region,
            app_id,
            secret_id,
            secret_key,
            bucket,
            avatar_bucket,
            domain,
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
            // 检查文件是否存在
            if !self.exists_by_name(&self.avatar_bucket, &name).await {
                if let Ok(data) = fs::read(&path).await {
                    self.upload_avatar(&name, data).await?;
                }
            }
        }
        Ok(())
    }

    // 构建腾讯云COS URL
    fn build_cos_url(&self, bucket: &str, key: &str) -> String {
        if !self.domain.is_empty() {
            // 如果配置了自定义域名，优先使用
            format!("{}/{}", self.domain.trim_end_matches('/'), key)
        } else {
            // 否则使用默认COS域名
            let bucket_with_appid = if self.app_id.is_empty() {
                bucket.to_string()
            } else {
                format!("{}-{}", bucket, self.app_id)
            };

            format!(
                "https://{}.cos.{}.myqcloud.com/{}",
                bucket_with_appid, self.region, key
            )
        }
    }

    // 生成COS API需要的Authorization签名
    async fn generate_auth_string(
        &self,
        method: &str,
        path: &str,
        query_params: Option<HashMap<String, String>>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<String, Error> {
        // 准备计算签名所需的参数
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("Failed to get timestamp: {}", e)))?
            .as_secs();

        // 签名有效期，默认60分钟
        let expiration = now + 3600;

        // 格式化路径，确保以/开头
        let formatted_path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        // 构建标准请求字符串
        let canonical_headers = headers.as_ref().map_or("".to_string(), |h| {
            let mut sorted_headers: Vec<_> = h.iter().collect();
            sorted_headers.sort_by(|a, b| a.0.cmp(b.0));

            sorted_headers
                .iter()
                .map(|(k, v)| format!("{}:{}\n", k.to_lowercase(), v.trim()))
                .collect::<Vec<_>>()
                .join("")
        });

        let signed_headers = headers.as_ref().map_or("".to_string(), |h| {
            let mut header_names: Vec<_> = h.keys().collect();
            header_names.sort();

            header_names
                .iter()
                .map(|k| k.to_lowercase())
                .collect::<Vec<_>>()
                .join(";")
        });

        // 构建查询字符串
        let canonical_query_string = query_params.as_ref().map_or("".to_string(), |q| {
            let mut sorted_params: Vec<_> = q.iter().collect();
            sorted_params.sort_by(|a, b| a.0.cmp(b.0));

            sorted_params
                .iter()
                .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&")
        });

        // 构建签名字符串
        let string_to_sign = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            method, formatted_path, canonical_query_string, canonical_headers, signed_headers
        );

        // 计算HMAC-SHA1签名
        let mut mac = HmacSha1::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(string_to_sign.as_bytes());
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());

        // 构建Authorization头
        let authorization = format!(
            "q-sign-algorithm=sha1&q-ak={}&q-sign-time={};{}&q-key-time={};{}&q-header-list={}&q-url-param-list={}&q-signature={}",
            self.secret_id,
            now, expiration,
            now, expiration,
            signed_headers,
            canonical_query_string,
            signature
        );

        Ok(authorization)
    }

    // 检查对象是否存在（调用HEAD Object API）
    async fn exists_by_name(&self, bucket: &str, key: &str) -> bool {
        let url = self.build_cos_url(bucket, key);

        match self.http_client.head(&url).send().await {
            Ok(response) => response.status() == StatusCode::OK,
            Err(_) => false,
        }
    }

    // 上传文件到COS（调用PUT Object API）
    async fn upload(&self, bucket: &str, key: &str, content: Vec<u8>) -> Result<(), Error> {
        let url = self.build_cos_url(bucket, key);

        // 准备请求头
        let mut headers = HashMap::new();
        headers.insert(
            "Content-Type".to_string(),
            "application/octet-stream".to_string(),
        );
        headers.insert("Content-Length".to_string(), content.len().to_string());
        headers.insert("Content-MD5".to_string(), calculate_md5(&content));

        // 生成授权签名
        let auth = self
            .generate_auth_string("PUT", key, None, Some(headers.clone()))
            .await?;

        // 创建HeaderMap用于reqwest
        let mut header_map = HeaderMap::new();
        // 添加Authorization头
        header_map.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&auth)
                .map_err(|e| Error::Internal(format!("Invalid header value: {}", e)))?,
        );

        // 添加其他请求头
        for (name, value) in headers {
            if let Ok(header_name) = HeaderName::from_str(&name) {
                if let Ok(header_value) = HeaderValue::from_str(&value) {
                    header_map.insert(header_name, header_value);
                }
            }
        }

        // 发送请求
        let response = self
            .http_client
            .put(&url)
            .headers(header_map)
            .body(content)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to upload file: {}", e)))?;

        if response.status().is_success() {
            info!(
                "Successfully uploaded file to COS: bucket={}, key={}",
                bucket, key
            );
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read response body".to_string());
            error!(
                "Failed to upload file to COS: status={}, body={}",
                status, body
            );
            Err(Error::Internal(format!(
                "Failed to upload file to COS: {}",
                status
            )))
        }
    }

    // 从COS下载文件（调用GET Object API）
    async fn download(&self, bucket: &str, key: &str) -> Result<Bytes, Error> {
        let url = self.build_cos_url(bucket, key);

        // 生成授权签名
        let auth = self.generate_auth_string("GET", key, None, None).await?;

        // 发送请求
        let response = self
            .http_client
            .get(&url)
            .header("Authorization", auth)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to download file: {}", e)))?;

        if response.status().is_success() {
            let bytes = response
                .bytes()
                .await
                .map_err(|e| Error::Internal(format!("Failed to read response body: {}", e)))?;

            info!(
                "Successfully downloaded file from COS: bucket={}, key={}, size={}",
                bucket,
                key,
                bytes.len()
            );
            Ok(bytes)
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read response body".to_string());
            error!(
                "Failed to download file from COS: status={}, body={}",
                status, body
            );
            Err(Error::Internal(format!(
                "Failed to download file from COS: {}",
                status
            )))
        }
    }

    // 从COS删除文件（调用DELETE Object API）
    async fn delete(&self, bucket: &str, key: &str) -> Result<(), Error> {
        let url = self.build_cos_url(bucket, key);

        // 生成授权签名
        let auth = self.generate_auth_string("DELETE", key, None, None).await?;

        // 发送请求
        let response = self
            .http_client
            .delete(&url)
            .header("Authorization", auth)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to delete file: {}", e)))?;

        if response.status().is_success() {
            info!(
                "Successfully deleted file from COS: bucket={}, key={}",
                bucket, key
            );
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read response body".to_string());
            error!(
                "Failed to delete file from COS: status={}, body={}",
                status, body
            );
            Err(Error::Internal(format!(
                "Failed to delete file from COS: {}",
                status
            )))
        }
    }

    // 获取对象元数据（调用HEAD Object API）
    async fn get_object_metadata(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<HashMap<String, String>, Error> {
        let url = self.build_cos_url(bucket, key);

        // 生成授权签名
        let auth = self.generate_auth_string("HEAD", key, None, None).await?;

        // 发送请求
        let response = self
            .http_client
            .head(&url)
            .header("Authorization", auth)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to get object metadata: {}", e)))?;

        if response.status().is_success() {
            // 提取响应头信息
            let headers = response.headers();
            let mut metadata = HashMap::new();

            for (name, value) in headers.iter() {
                if let Ok(value_str) = value.to_str() {
                    metadata.insert(name.as_str().to_string(), value_str.to_string());
                }
            }

            info!(
                "Successfully got object metadata from COS: bucket={}, key={}",
                bucket, key
            );
            Ok(metadata)
        } else {
            let status = response.status();
            error!("Failed to get object metadata from COS: status={}", status);
            Err(Error::Internal(format!(
                "Failed to get object metadata from COS: {}",
                status
            )))
        }
    }

    // 生成预签名URL（用于前端直接上传）
    async fn generate_presigned_url(
        &self,
        bucket: &str,
        key: &str,
        method: &str,
        content_type: &str,
        expiration: Duration,
    ) -> Result<String, Error> {
        // 计算签名所需的时间戳
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("Failed to get timestamp: {}", e)))?
            .as_secs();

        // 计算过期时间
        let expiry = now + expiration.as_secs();

        // 准备签名参数
        let key_time = format!("{};{}", now, expiry);

        // 构建要签名的字符串
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), content_type.to_string());

        // 签名字段
        let header_list = "content-type";
        let param_list = "";

        // 构建HTTP请求字符串
        let http_string = format!("{}\n/{}\n\ncontent-type={}\n\n", method, key, content_type);

        // 计算StringToSign
        let string_to_sign = format!("sha1\n{}\n{}\n", key_time, http_string);

        // 计算SignKey
        let mut mac = HmacSha1::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(key_time.as_bytes());
        let sign_key = format!("{:x}", mac.finalize().into_bytes());

        // 计算Signature
        let mut mac = HmacSha1::new_from_slice(sign_key.as_bytes())
            .map_err(|e| Error::Internal(format!("Failed to create HMAC: {}", e)))?;
        mac.update(string_to_sign.as_bytes());
        let signature = format!("{:x}", mac.finalize().into_bytes());

        // 构建完整的预签名URL
        let bucket_with_appid = if self.app_id.is_empty() {
            bucket.to_string()
        } else {
            format!("{}-{}", bucket, self.app_id)
        };

        let mut url_string = if !self.domain.is_empty() {
            format!("{}/{}", self.domain.trim_end_matches('/'), key)
        } else {
            format!(
                "https://{}.cos.{}.myqcloud.com/{}",
                bucket_with_appid, self.region, key
            )
        };

        // 添加查询参数
        url_string = format!(
            "{}?q-sign-algorithm=sha1&q-ak={}&q-sign-time={}&q-key-time={}&q-header-list={}&q-url-param-list={}&q-signature={}",
            url_string, self.secret_id, key_time, key_time, header_list, param_list, signature
        );

        info!(
            "Generated presigned URL for COS: bucket={}, key={}, method={}",
            bucket, key, method
        );
        Ok(url_string)
    }
}

#[async_trait]
impl Oss for CosClient {
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
        // 为PUT操作生成预签名URL
        self.generate_presigned_url(&self.bucket, key, "put", content_type, expiration)
            .await
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
            error!(
                "Size mismatch for {}: expected {}, got {}",
                key, expected_size, content_length
            );
            return Ok(false);
        }

        // 验证MD5（如果有）
        if let Some(etag) = metadata.get("etag") {
            // 腾讯云COS的ETag通常是MD5值，但可能带有双引号
            let etag_value = etag.trim_matches('"');

            if etag_value != expected_md5 {
                error!(
                    "MD5 mismatch for {}: expected {}, got {}",
                    key, expected_md5, etag_value
                );
                return Ok(false);
            }
        } else {
            // 如果没有ETag，则下载文件并计算MD5
            let content = self.download(&self.bucket, key).await?;
            let actual_md5 = calculate_md5(&content);

            if actual_md5 != expected_md5 {
                error!(
                    "MD5 mismatch for {}: expected {}, got {}",
                    key, expected_md5, actual_md5
                );
                return Ok(false);
            }
        }

        info!("Upload validation successful for {}", key);
        Ok(true)
    }
}
