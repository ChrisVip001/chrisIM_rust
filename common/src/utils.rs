use crate::{Error, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use image::{DynamicImage, ImageBuffer, Rgb};
use redis::{Client as RedisClient, Commands};
use imageproc::drawing::draw_text_mut;
use rand::distr::Alphanumeric;
use rand::Rng;
use uuid::Uuid;
use regex::Regex;
use rusttype::{Font, Scale};
use crate::config::ConfigLoader;
// 导入雪花ID模块
use crate::snowflake::SNOWFLAKE;

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

/// 使用雪花算法生成用户ID
pub fn generate_user_id() -> Result<String> {
    // 最多重试3次
    for _ in 0..3 {
        match SNOWFLAKE.generate() {
            Ok(id) => return Ok(id.to_string()),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(1)), // 短暂延迟后重试
        }
    }
    
    // 如果多次重试仍然失败，返回错误
    Err(Error::Internal("无法生成雪花ID，请检查系统时钟".to_string()))
}



pub fn generate_user_custom_id() -> String {
    let mut rng = rand::thread_rng();

    // 生成8位字母和数字的随机字符串
    let random_id: String = (0..8)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            match idx {
                0..=9 => (b'0' + idx as u8) as char,   // 数字 0-9
                10..=35 => (b'a' + (idx - 10) as u8) as char,  // 小写字母 a-z
                _ => (b'A' + (idx - 36) as u8) as char,  // 大写字母 A-Z
            }
        })
        .collect();

    // 拼接前缀
    format!("myid-{}", random_id)
}

/// 图片验证码生成
pub fn generate_captcha_image(width: &u32, height: &u32, text_code: &str, font_size: &f32) -> Vec<u8> {
    // 图片尺寸
    let width = width;
    let height = height;

    // 创建一个白色背景的图片（使用 Rgb<u8>）
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(*width, *height, Rgb([255, 255, 255]));

    // 使用 DynamicImage 包装
    let mut img = DynamicImage::ImageRgb8(img);

    // 加载字体文件
    let font_data = include_bytes!("../assets/Roboto-Regular.ttf"); // 确保路径正确
    let font = Font::try_from_bytes(font_data).expect("加载字体失败");

    // 设置字体大小和颜色
    let scale = Scale::uniform(font_size.clone());
    let color = Rgb([0, 0, 0]); // 黑色

    // 在图片上绘制文本
    draw_text_mut(img.as_mut_rgb8().unwrap(),color,10,10,scale,&font,text_code);

    // 将图片转换为字节流
    let mut buffer = Vec::new();
    img.write_to(&mut buffer, image::ImageOutputFormat::Png)
        .expect("写入图片失败");

    buffer

}

/// 生成随机验证码文本
pub fn generate_captcha_text() -> String {
    let mut rng = rand::thread_rng();
    (0..6) // 生成 6 位数字
        .map(|_| rng.gen_range(0..10).to_string())
        .collect::<Vec<_>>()
        .join("")
}

/// 图片验证码保存
pub fn save_image_code(code_str: &str, captcha_text: &str, expire_time: &u32) -> () {
    // 获取配置
    let config = ConfigLoader::get_global().expect("获取全局配置失败");
    // 创建Redis客户端
    let redis_url = config.redis.url();
    let mut redis_client = RedisClient::open(redis_url).expect("创建Redis客户端失败");
    // 存储验证码到redis,有效时间expire_time秒
    redis_client.set_ex::<&str, &str, ()>(code_str, captcha_text, *expire_time as u64)
        .expect("存储验证码到Redis失败");
}

/// 图片验证码校验
const IMAGE_CODE_PREFIX: &str = "image:verification:code";
pub fn verify_image_code(code_key: &str, input_code: &str) -> bool {
    // 获取配置
    let config = ConfigLoader::get_global().expect("获取全局配置失败");
    // 创建Redis客户端
    let redis_url = config.redis.url();
    let mut redis_client = RedisClient::open(redis_url).expect("创建Redis客户端失败");
    let get_code_key = format!("{}:{}", IMAGE_CODE_PREFIX, code_key);
    let redis_code_value: String = redis_client.get(&get_code_key).unwrap_or("0".to_string());
    if redis_code_value == input_code {
        // 匹配成功，删除redis中存储数据
        let del_stat: () = redis_client.del(get_code_key).unwrap_or(());
        true
    } else {
        false
    }
}





