use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use async_trait::async_trait;
use redis::{AsyncCommands, Client};
use serde_json::Value;
use tracing::{debug, error, info};
use rand::Rng;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest;
use serde::{Deserialize, Serialize};
use crate::configs::SmsConfig;
use crate::Result;
use crate::Error;
use crate::sms::SmsService;

const SMS_VERIFICATION_CODE_PREFIX: &str = "sms:verification:code:";

/// 腾讯云短信参数
#[derive(Debug, Serialize)]
struct TencentSmsParams {
    #[serde(rename = "PhoneNumberSet")]
    phone_number_set: Vec<String>,
    #[serde(rename = "SmsSdkAppId")]
    sms_sdk_app_id: String,
    #[serde(rename = "TemplateId")]
    template_id: String,
    #[serde(rename = "SignName")]
    sign_name: String,
    #[serde(rename = "TemplateParamSet")]
    template_param_set: Vec<String>,
}

/// 腾讯云短信服务实现
pub struct TencentSmsService {
    redis_client: Client,
    config: Arc<SmsConfig>,
    http_client: reqwest::Client,
}

impl TencentSmsService {
    pub fn new(redis_client: Client, config: Arc<SmsConfig>) -> Self {
        Self {
            redis_client,
            config,
            http_client: reqwest::Client::new(),
        }
    }

    /// 生成随机验证码
    fn generate_code(&self) -> String {
        let mut rng = rand::thread_rng();
        let code_length = self.config.tencent.code_length as usize;
        
        (0..code_length)
            .map(|_| rng.gen_range(0..10).to_string())
            .collect()
    }

    /// 构造Redis中验证码的键名
    fn get_redis_key(&self, phone: &str) -> String {
        format!("{}{}", SMS_VERIFICATION_CODE_PREFIX, phone)
    }
    
    /// 构造腾讯云API签名 - V3版本签名
    fn generate_signature(&self, timestamp: u64, payload: &str) -> String {
        // 1. 获取UTC日期（格式：2019-01-01）用于请求头和凭证
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        
        // 2. 确定服务参数
        let service = "sms";
        let host = "sms.tencentcloudapi.com";
        
        // 3. 拼接规范请求串
        // 3.1 拼接谓词和URI（通常为"POST"、"/"）
        let http_request_method = "POST";
        let canonical_uri = "/";
        
        // 3.2 拼接规范查询字符串（一般API请求为空）
        let canonical_querystring = "";
        
        // 3.3 拼接规范标头
        let canonical_headers = format!("content-type:application/json; charset=utf-8\nhost:{}\n", host);
        
        // 3.4 签名标头列表
        let signed_headers = "content-type;host";
        
        // 3.5 计算请求正文哈希
        let payload_hash = {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(payload.as_bytes());
            format!("{:x}", hasher.finalize())
        };
        
        // 拼接完整的规范请求
        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            http_request_method,
            canonical_uri,
            canonical_querystring,
            canonical_headers,
            signed_headers,
            payload_hash
        );
        
        debug!("步骤3 - 规范请求串:\n{}", canonical_request);
        
        // 4. 计算规范请求串哈希
        let canonical_request_hash = {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(canonical_request.as_bytes());
            format!("{:x}", hasher.finalize())
        };
        
        // 5. 拼接待签名字符串
        let algorithm = "TC3-HMAC-SHA256";
        let credential_scope = format!("{}/{}/tc3_request", date, service);
        
        let string_to_sign = format!(
            "{}\n{}\n{}\n{}",
            algorithm,
            timestamp,
            credential_scope,
            canonical_request_hash
        );
        
        debug!("步骤5 - 待签名字符串:\n{}", string_to_sign);
        
        // 6. 计算签名
        // 6.1 派生签名密钥
        let secret_date = hmac_sha256(format!("TC3{}", self.config.tencent.secret_key).as_bytes(), date.as_bytes());
        let secret_service = hmac_sha256(&secret_date, service.as_bytes());
        let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
        
        // 6.2 计算签名
        let signature = hmac_sha256_hex(&secret_signing, string_to_sign.as_bytes());
        
        debug!("步骤6 - 签名结果: {}", signature);
        
        // 7. 拼接授权字符串
        let authorization = format!(
            "{} Credential={}/{}, SignedHeaders={}, Signature={}",
            algorithm,
            self.config.tencent.secret_id,
            credential_scope,
            signed_headers,
            signature
        );
        
        debug!("步骤7 - 完整授权字符串: {}", authorization);
        
        authorization
    }
}

// HMAC-SHA256函数
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC初始化失败");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

// HMAC-SHA256结果转十六进制
fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let bytes = hmac_sha256(key, data);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[async_trait]
impl SmsService for TencentSmsService {
    async fn send_verification_code(&self, phone: &str) -> Result<String> {
        // 获取Redis连接
        let mut conn = self.redis_client.get_async_connection().await
            .map_err(|e| Error::Redis(format!("获取Redis连接失败: {}", e)))?;
        
        // 检查Redis中是否已存在验证码
        let redis_key = self.get_redis_key(phone);
        
        // 如果启用了防重复发送
        if self.config.tencent.throttle_enabled {
            // 检查是否存在验证码
            let existing_code: Option<String> = conn.get(&redis_key).await
                .map_err(|e| Error::Redis(format!("从Redis获取验证码失败: {}", e)))?;
            
            // 如果验证码存在，检查是否在限制时间内
            if let Some(code) = existing_code {
                // 获取验证码剩余过期时间(秒)
                let ttl: i64 = conn.ttl(&redis_key).await
                    .map_err(|e| Error::Redis(format!("获取验证码过期时间失败: {}", e)))?;
                
                // 计算验证码已存在的时间(秒)
                let elapsed_seconds = self.config.tencent.expire_seconds as i64 - ttl;
                
                // 如果验证码已存在的时间小于限制时间，则不允许重发
                if ttl > 0 && elapsed_seconds < self.config.tencent.throttle_seconds as i64 {
                    let remaining_seconds = self.config.tencent.throttle_seconds as i64 - elapsed_seconds;
                    debug!("手机号 {} 的验证码 {} 发送过于频繁，请在 {} 秒后重试", phone, code, remaining_seconds);
                    return Err(Error::Sms(format!("发送验证码过于频繁，请在 {} 秒后重试", remaining_seconds)));
                }
                
                // 如果已超过限制时间，可以重新发送，删除旧验证码
                let _: () = conn.del(&redis_key).await
                    .map_err(|e| Error::Redis(format!("删除旧验证码失败: {}", e)))?;
                
                debug!("手机号 {} 的旧验证码已超过限制时间，允许重新发送", phone);
            }
        }
        
        // 生成新的验证码
        let code = self.generate_code();
        debug!("为手机号 {} 生成验证码: {}", phone, code);
        
        // 确保手机号格式正确（腾讯云SMS要求E.164格式：+国家代码手机号）
        let formatted_phone = if phone.starts_with("+") {
            phone.to_string()
        } else if phone.starts_with("86") {
            format!("+{}", phone)
        } else {
            format!("+86{}", phone)
        };
        
        debug!("格式化后的手机号: {}", formatted_phone);
        
        // 将验证码存入Redis，设置过期时间
        let expire_seconds = self.config.tencent.expire_seconds;
        
        conn.set_ex(&redis_key, &code, expire_seconds).await
            .map_err(|e| Error::Redis(format!("存储验证码到Redis失败: {}", e)))?;
        
        // 构建API请求参数
        let params = TencentSmsParams {
            phone_number_set: vec![formatted_phone],
            sms_sdk_app_id: self.config.tencent.app_id.clone(),
            template_id: self.config.tencent.template_id.clone(),
            sign_name: self.config.tencent.sign_name.clone(),
            template_param_set: vec![code.clone()],
        };
        
        // 获取当前时间戳
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("获取时间失败")
            .as_secs();
        
        debug!("使用的时间戳: {}", timestamp);
        
        // 将请求参数转为JSON
        let payload = serde_json::to_string(&params)
            .map_err(|e| Error::Sms(format!("序列化请求参数失败: {}", e)))?;
            
        // 生成签名
        let authorization = self.generate_signature(timestamp, &payload);
        
        // 记录请求信息
        debug!("腾讯云短信API请求参数: {:?}", params);
        debug!("腾讯云短信API请求体: {}", payload);
        debug!("腾讯云短信API签名: {}", authorization);
        
        // 发送HTTP请求
        let response = self.http_client
            .post("https://sms.tencentcloudapi.com")
            .header("Authorization", authorization)
            .header("Content-Type", "application/json; charset=utf-8")
            .header("Host", "sms.tencentcloudapi.com")
            .header("X-TC-Action", "SendSms")
            .header("X-TC-Version", "2021-01-11")
            .header("X-TC-Timestamp", timestamp.to_string())
            .header("X-TC-Region", self.config.tencent.region.clone())
            .body(payload) // 使用body而不是json，因为我们已经序列化了请求体
            .send()
            .await
            .map_err(|e| Error::Sms(format!("发送短信API请求失败: {}", e)))?;
            
        // 获取响应内容
        let status_code = response.status();
        debug!("HTTP响应状态码: {}", status_code);
        
        let response_text = response.text().await
            .map_err(|e| Error::Sms(format!("读取响应内容失败: {}", e)))?;
            
        debug!("腾讯云短信API原始响应: {}", response_text);
        
        // 解析响应JSON
        match serde_json::from_str::<Value>(&response_text) {
            Ok(json) => {
                debug!("解析响应JSON成功: {:?}", json);
                
                // 检查是否有错误信息
                if let Some(response) = json.get("Response") {
                    if let Some(error) = response.get("Error") {
                        let error_code = error.get("Code").and_then(|c| c.as_str()).unwrap_or("UnknownError");
                        let error_message = error.get("Message").and_then(|m| m.as_str()).unwrap_or("未知错误");
                        error!("腾讯云API返回错误: [{}] {}", error_code, error_message);
                        return Err(Error::Sms(format!("腾讯云API错误: [{}] {}", error_code, error_message)));
                    }
                    
                    // 检查响应成功
                    if response.get("RequestId").is_some() {
                        // 检查发送状态
                        if let Some(status_set) = response.get("SendStatusSet").or_else(|| response.get("sendStatusSet")) {
                            if let Some(status_array) = status_set.as_array() {
                                if !status_array.is_empty() {
                                    let status = &status_array[0];
                                    
                                    // 获取发送状态码
                                    let status_code = status.get("Code").or_else(|| status.get("code"))
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("");
                                        
                                    if status_code == "Ok" || status_code == "ok" || status_code == "SUCCESS" || status_code.is_empty() {
                                        info!("短信验证码发送成功，手机号: {}", phone);
                                        return Ok(code);
                                    } else {
                                        let message = status.get("Message").or_else(|| status.get("message"))
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("未知错误");
                                        error!("短信发送失败: {}", message);
                                        return Err(Error::Sms(format!("短信发送失败: {}", message)));
                                    }
                                }
                            }
                        }
                        
                        // 如果没有明确的错误但有RequestId，认为请求成功
                        info!("短信验证码发送处理完成，手机号: {}", phone);
                        return Ok(code);
                    }
                }
                
                // 如果响应格式不符合预期但没有明确错误
                info!("短信API响应格式不符合预期，但无明确错误。假设发送成功，手机号: {}", phone);
                Ok(code)
            },
            Err(e) => {
                error!("解析响应JSON失败: {}，原始响应: {}", e, response_text);
                Err(Error::Sms(format!("解析响应JSON失败: {}", e)))
            }
        }
    }

    async fn verify_code(&self, phone: &str, code: &str) -> Result<bool> {
        // 从Redis获取存储的验证码
        let mut conn = self.redis_client.get_async_connection().await
            .map_err(|e| Error::Redis(format!("获取Redis连接失败: {}", e)))?;
        
        let redis_key = self.get_redis_key(phone);
        
        // 获取存储的验证码
        let stored_code: Option<String> = conn.get(&redis_key).await
            .map_err(|e| Error::Redis(format!("从Redis获取验证码失败: {}", e)))?;
        
        match stored_code {
            Some(stored) => {
                // 验证码匹配成功后，删除Redis中的验证码（一次性使用）
                if stored == code {
                    // 删除验证码
                    let _: () = conn.del(&redis_key).await
                        .map_err(|e| Error::Redis(format!("删除Redis验证码失败: {}", e)))?;
                    
                    info!("验证码验证成功，手机号: {}", phone);
                    Ok(true)
                } else {
                    debug!("验证码不匹配，手机号: {}, 输入: {}, 存储: {}", phone, code, stored);
                    Ok(false)
                }
            },
            None => {
                debug!("验证码不存在或已过期，手机号: {}", phone);
                Ok(false)
            }
        }
    }
} 