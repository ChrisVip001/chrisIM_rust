use crate::{Error, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use rand::distr::Alphanumeric;
use rand::Rng;
use uuid::Uuid;
use regex::Regex;

/// 生成随机盐值用于密码哈希
pub fn generate_salt() -> String {
    SaltString::generate(&mut OsRng).to_string()
}

/// 使用Argon2算法对密码进行哈希处理
pub fn argon2_hash_password(password: &[u8], salt: &str) -> std::result::Result<String, Error> {
    // 使用默认的Argon2配置
    // 这个配置可以更改为适合您具体安全需求和性能要求的设置

    // 使用默认参数的Argon2 (Argon2id v19)
    let argon2 = Argon2::default();

    // 将密码哈希为PHC字符串 ($argon2id$v=19$...)
    Ok(argon2
        .hash_password(password, &SaltString::from_b64(salt).unwrap())
        .map_err(|e| Error::Internal(e.to_string()))?
        .to_string())
}

// 密码哈希工具
pub fn hash_password(password: &str) -> Result<String> {
    let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| Error::Internal(format!("密码哈希失败: {}", e)))?;
    Ok(hashed)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let is_valid = bcrypt::verify(password, hash)
        .map_err(|e| Error::Internal(format!("密码验证失败: {}", e)))?;
    Ok(is_valid)
}

pub fn validate_phone(phone: &str) -> bool {
    Regex::new(r"^1[3-9]\d{9}$")
        .expect("手机号正则表达式编译失败")
        .is_match(phone)
}

pub fn url(https: bool, host: &str, port: u16) -> String {
    if https {
        format!("https://{}:{}", host, port)
    } else {
        format!("http://{}:{}", host, port)
    }
}

pub fn wss_url(wss: bool, host: &str, port: u16) -> String {
    if wss {
        format!("wss://{}:{}", host, port)
    } else {
        format!("ws://{}:{}", host, port)
    }
}

/// 获取主机名
///
/// 返回当前机器的主机名，如果获取失败则返回错误
pub fn get_host_name() -> Result<String> {
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .map_err(|_| Error::Internal("获取主机名失败".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{PasswordHash, PasswordVerifier};
    use rs_consul::{Config, Consul, GetServiceNodesRequest};

    /// 测试密码哈希功能
    #[test]
    fn test_hash_password() {
        let salt = generate_salt();
        let password = "123456";
        let hash = argon2_hash_password(password.as_bytes(), &salt).unwrap();
        let parsed_hash = PasswordHash::new(&hash).unwrap();
        assert!(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok());
    }
}


pub fn generate_user_id() -> String {
    let uuid = Uuid::new_v4().simple(); // 生成32位的UUID（无连字符）
    let mut rng = rand::rng();

    // 取UUID的前16位，并补充6位随机字母和数字
    let prefix = &uuid.to_string()[..16];
    let suffix: String = (0..6)
        .map(|_| rng.sample(Alphanumeric) as char)
        .collect();

    format!("{}{}", prefix, suffix)
}

