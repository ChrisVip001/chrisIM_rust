use contract::{Error, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use image::{DynamicImage, ImageBuffer, Rgba};
use redis::{Client as RedisClient, Commands};
use rand::Rng;
use regex::Regex;
use crate::config::AppConfig;
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
    let mut rng = rand::rng();

    // 生成8位字母和数字的随机字符串
    let random_id: String = (0..8)
        .map(|_| {
            let idx = rng.random_range(0..62);
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

/// 5x7 数字点阵。每行取低 5 位，最高位是最左列。
///
/// 验证码文本只有数字（见 [`generate_captcha_text`]），所以内嵌字模即可，
/// 不需要字体解析库 —— 那会把 `ttf-parser`（已标记不维护）带进依赖图。
const DIGIT_GLYPHS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110], // 0
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110], // 1
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111], // 2
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110], // 3
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010], // 4
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110], // 5
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110], // 6
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000], // 7
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110], // 8
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100], // 9
];

/// 在画布上绘制放大后的点阵数字。(x, y) 为左上角，`scale` 是像素放大倍数，
/// `shear` 是每行右移的像素数（模拟倾斜），越界的像素自动丢弃。
fn draw_digit_glyph(
    img: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
    x: i32,
    y: i32,
    scale: i32,
    digit: usize,
    color: Rgba<u8>,
    shear: i32,
) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let glyph = DIGIT_GLYPHS[digit.min(9)];
    for (row, bits) in glyph.iter().enumerate() {
        let row_shift = shear * row as i32 / 7;
        for col in 0..5i32 {
            if bits & (1 << (4 - col)) == 0 {
                continue;
            }
            let px0 = x + col * scale + row_shift;
            let py0 = y + row as i32 * scale;
            for dy in 0..scale {
                for dx in 0..scale {
                    let (px, py) = (px0 + dx, py0 + dy);
                    if px >= 0 && py >= 0 && px < w && py < h {
                        img.put_pixel(px as u32, py as u32, color);
                    }
                }
            }
        }
    }
}

/// Bresenham 直线，用于验证码干扰线（替代 imageproc 的绘制函数）。
fn draw_rough_line(
    img: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Rgba<u8>,
) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        if x >= 0 && y >= 0 && x < w && y < h {
            img.put_pixel(x as u32, y as u32, color);
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// 图片验证码生成
pub fn generate_captcha_image(width: &u32, height: &u32, text_code: &str, font_size: &f32) -> Vec<u8> {
    // 图片尺寸
    let width = *width;
    let height = *height;

    // 创建一个纯白色的背景图片
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(width, height);
    for pixel in img.pixels_mut() {
        *pixel = Rgba([255, 255, 255, 255]); // 纯白色背景
    }

    // 添加轻微噪点
    let mut rng = rand::rng();
    for _ in 0..500 {
        let x = rng.random_range(0..width);
        let y = rng.random_range(0..height);
        let color = Rgba([
            rng.random_range(180..220), // 浅灰色噪点
            rng.random_range(180..220),
            rng.random_range(180..220),
            255,
        ]);
        img.put_pixel(x, y, color);
    }

    // 点阵字号：5x7 字模，按传入字号换算整数放大倍数，并限制在画布高度内
    let char_count = text_code.chars().count().max(1) as i32;
    // 放大倍数同时受高度（占 60%）与宽度（每字符含间距）约束，避免字符被裁切
    let max_scale_h = ((height as i32) * 6 / 10 / 7).max(2);
    let max_scale_w = (((width as i32) / char_count) / 6).max(2);
    let scale = ((*font_size / 7.0).round() as i32).clamp(2, max_scale_h.min(max_scale_w));
    let glyph_w = 5 * scale;
    let glyph_h = 7 * scale;
    // 字符间距：均分剩余空间，最少 2px、最多 20px
    let gap = ((width as i32 - glyph_w * char_count) / (char_count + 1)).clamp(2, 20);
    let mut x_offset = gap;

    for c in text_code.chars() {
        // 随机字体颜色
        let color = Rgba([
            rng.random_range(0..150),       // 红色分量
            rng.random_range(0..150),       // 绿色分量
            rng.random_range(0..150),       // 蓝色分量
            255,                         // 完全不透明
        ]);

        // 随机上下偏移 + 轻微倾斜：抗 OCR，但不影响肉眼识别
        let max_y = (height as i32 - glyph_h).max(0);
        let y_offset = if max_y > 0 { rng.random_range(0..=max_y / 2) } else { 0 };
        let shear = rng.random_range(-scale..=scale);
        draw_digit_glyph(
            &mut img,
            x_offset,
            y_offset,
            scale,
            c.to_digit(10).unwrap_or(0) as usize,
            color,
            shear,
        );

        x_offset += glyph_w + gap;
    }

    // 添加简单装饰线条
    for _ in 0..3 {
        let start_x = rng.random_range(0..width as i32);
        let start_y = rng.random_range(0..height as i32);
        let end_x = rng.random_range(0..width as i32);
        let end_y = rng.random_range(0..height as i32);
        let line_color = Rgba([
            rng.random_range(150..200), // 浅灰色线条
            rng.random_range(150..200),
            rng.random_range(150..200),
            255,
        ]);
        draw_rough_line(&mut img, start_x, start_y, end_x, end_y, line_color);
    }

    // 将图片转换为字节流
    let mut cursor = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("写入图片失败");

    cursor.into_inner()

}

/// 生成随机验证码文本
pub fn generate_captcha_text() -> String {
    let mut rng = rand::rng();
    (0..6) // 生成 6 位数字
        .map(|_| rng.random_range(0..10).to_string())
        .collect::<Vec<_>>()
        .join("")
}

/// 图片验证码保存
///
/// 配置由调用方传入，工具函数不再回读全局单例。
pub fn save_image_code(
    config: &AppConfig,
    code_str: &str,
    captcha_text: &str,
    expire_time: &u32,
) -> () {
    // 创建Redis客户端
    let redis_url = config.redis.url();
    let mut redis_client = RedisClient::open(redis_url).expect("创建Redis客户端失败");
    // 存储验证码到redis,有效时间expire_time秒
    redis_client.set_ex::<&str, &str, ()>(code_str, captcha_text, *expire_time as u64)
        .expect("存储验证码到Redis失败");
}

/// 图片验证码校验
const IMAGE_CODE_PREFIX: &str = "image:verification:code";
pub fn verify_image_code(config: &AppConfig, code_key: &str, input_code: &str) -> bool {
    // 创建Redis客户端
    let redis_url = config.redis.url();
    let mut redis_client = RedisClient::open(redis_url).expect("创建Redis客户端失败");
    let get_code_key = format!("{}:{}", IMAGE_CODE_PREFIX, code_key);
    let redis_code_value: String = redis_client.get(&get_code_key).unwrap_or("0".to_string());
    if redis_code_value == input_code {
        // 匹配成功，删除redis中存储数据
        let _: () = redis_client.del(get_code_key).unwrap_or(());
        true
    } else {
        false
    }
}

#[cfg(test)]
mod captcha_tests {
    use super::*;

    #[test]
    fn captcha_image_is_valid_png() {
        let text = generate_captcha_text();
        assert_eq!(text.len(), 6);
        let png = generate_captcha_image(&120, &40, &text, &28.0);
        assert!(png.len() > 500, "验证码图片过小: {} 字节", png.len());
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        // 供人工抽查渲染效果（只在测试中写临时目录）
        let _ = std::fs::write(std::env::temp_dir().join("captcha_sample.png"), &png);
    }

    #[test]
    fn captcha_image_survives_odd_sizes() {
        // 极小字号、超大字号、零字号都不能 panic（点阵绘制有越界保护）
        for (w, h, fs) in [(40u32, 16u32, 6.0f32), (300, 120, 100.0), (120, 40, 0.0)] {
            let png = generate_captcha_image(&w, &h, "012345", &fs);
            assert!(!png.is_empty(), "尺寸 {w}x{h} 字号 {fs} 生成为空");
        }
    }
}





