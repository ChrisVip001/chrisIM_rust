use crate::{models::Claims, Error, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use std::env;
use rand::distr::Alphanumeric;
use rand::Rng;
use uuid::Uuid;
use regex::Regex;

// JWT工具函数
pub fn generate_jwt(user_id: &Uuid, username: &str) -> Result<String> {
    let expiration = Utc::now()
        .checked_add_signed(Duration::seconds(
            env::var("JWT_EXPIRATION")
                .unwrap_or_else(|_| "86400".to_string())
                .parse()
                .unwrap_or(86400),
        ))
        .expect("有效的时间戳")
        .timestamp() as usize;

    let claims = Claims {
        sub: user_id.to_string(),
        username: username.to_string(),
        exp: expiration,
        iat: Utc::now().timestamp() as usize,
    };

    let secret = env::var("JWT_SECRET").unwrap_or_else(|_| "default_jwt_secret".to_string());
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

pub fn validate_jwt(token: &str) -> Result<Claims> {
    let secret = env::var("JWT_SECRET").unwrap_or_else(|_| "default_jwt_secret".to_string());
    let validation = Validation::default();
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(token_data.claims)
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
    Regex::new(r"^1[3-9]\d{9}$").expect("手机号正则表达式编译失败").is_match(phone)
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

