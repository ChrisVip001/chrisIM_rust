use axum::{
    body::Body,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use prost_types::Timestamp;
use serde_json::{json, Value};
use regex::Regex;
use once_cell::sync::Lazy;

// 预编译正则表达式，匹配gRPC错误消息格式
static GRPC_ERROR_REGEX: Lazy<Regex> = Lazy::new(|| {
    // 匹配两种可能的格式：带转义的引号和不带转义的引号
    Regex::new(r#"message: (?:\\"|")(.+?)(?:\\"|")"#).unwrap()
});

/// 通用响应生成辅助函数 - 成功响应
pub fn success_response<T: serde::Serialize>(data: T, status_code: StatusCode) -> axum::response::Response<Body> {
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "data": data,
            "success": true
        })),
    ).into_response()
}

/// 通用响应生成辅助函数 - 成功带消息
pub fn success_with_message<T: serde::Serialize>(data: T, message: &str, status_code: StatusCode) -> axum::response::Response<Body> {
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "data": data,
            "message": message,
            "success": true
        })),
    ).into_response()
}

/// 通用响应生成辅助函数 - 错误响应
pub fn error_response(message: &str, status_code: StatusCode) -> axum::response::Response<Body> {
    // 检查是否为gRPC错误消息格式
    let processed_message = if (message.contains("message: \"") || message.contains("message: \\\"")) && 
                              (message.contains("\", details:") || message.contains("\\\", details:")) {
        // 从gRPC错误消息中提取主要错误信息
        if let Some(captures) = GRPC_ERROR_REGEX.captures(message) {
            if let Some(matched) = captures.get(1) {
                matched.as_str()
            } else {
                message
            }
        } else {
            message
        }
    } else {
        message
    };
    
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "message": processed_message,
            "success": false
        })),
    ).into_response()
}

/// 参数提取辅助函数 - 从JSON中提取字符串参数 （必填）
pub fn extract_string_param(body: &Value, param_name: &str, alt_name: Option<&str>) -> Result<String, anyhow::Error> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .map(|v| match v {
            Value::String(s) => s.to_string(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => String::new()
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("参数 {} 缺失或格式错误", param_name))
}

/// 参数提取辅助函数 - 从JSON中提取可选字符串参数 （非必填）
pub fn get_optional_string(body: &Value, param_name: &str, alt_name: Option<&str>) -> Option<String> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .and_then(|v| {
            if v.is_null() {
                None
            } else if v.is_string() {
                v.as_str().map(|s| s.to_string())
            } else if v.is_number() {
                Some(v.to_string())
            } else if v.is_boolean() {
                Some(v.as_bool().unwrap().to_string())
            } else {
                None
            }
        })
}

/// 参数提取辅助函数 - 从JSON中提取i64整数参数 （非必填给默认值）
pub fn get_i64_param(body: &Value, param_name: &str, default: i64) -> i64 {
    body.get(param_name)
        .and_then(|v| {
            if v.is_i64() {
                v.as_i64()
            } else if v.is_string() {
                v.as_str().and_then(|s| s.parse::<i64>().ok())
            } else {
                None
            }
        })
        .unwrap_or(default)
}

/// 参数提取辅助函数 - 从JSON中提取i64整数参数 （必填）
pub fn extract_i64_param(body: &Value, param_name: &str, alt_name: Option<&str>) -> Result<i64, anyhow::Error> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .and_then(|v| {
            if v.is_i64() || v.is_u64() {
                v.as_i64()
            } else if v.is_f64() {
                let f = v.as_f64().unwrap();
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            } else if v.is_string() {
                v.as_str().and_then(|s| s.parse::<i64>().ok())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("参数 {} 缺失或格式错误", param_name))
}

/// 时间戳转换为RFC3339格式的字符串
pub fn timestamp_to_rfc3339(timestamp: &Option<prost_types::Timestamp>) -> String {
    timestamp
        .as_ref()
        .map(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

// Timestamp 转 DateTime<Utc>
pub fn timestamp_to_datetime(ts: Option<Timestamp>) -> Option<DateTime<Utc>> {
    ts.map(|ts| {
        Utc.timestamp_opt(ts.seconds, ts.nanos as u32)
            .single()
            .unwrap_or_default()
    })
}

// DateTime<Utc> 转 Timestamp
pub fn datetime_to_timestamp(dt: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

// 格式化显示时间(yyyy-MM-dd HH:mm:ss)
pub fn format_timestamp(ts: Option<Timestamp>) -> String {
    if let Some(dt) = timestamp_to_datetime(ts) {
        // 创建一个 UTC+8 的固定偏移量
        let shanghai_offset = FixedOffset::east_opt(8 * 3600).unwrap(); // 8 小时 = 8 * 3600 秒
        // 将 UTC 时间转换为东八区时间
        let shanghai_time = dt.with_timezone(&shanghai_offset);
        shanghai_time.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        "".to_string()
    }
}

/// 时间戳转换为yyyy-MM-dd HH:mm:ss格式的字符串（东八区时间）
pub fn timestamp_to_datetime_string(timestamp: &Option<prost_types::Timestamp>) -> String {
    timestamp
        .as_ref()
        .map(|ts| {
            if ts.seconds < 0 {
                return String::new();
            }
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .map(|dt| {
                    // 转换为东八区时间
                    let beijing = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
                    dt.with_timezone(&beijing)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
}