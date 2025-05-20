use axum::{
    body::Body,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

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
    (
        status_code,
        Json(json!({
            "code": status_code.as_u16(),
            "message": message,
            "success": false
        })),
    ).into_response()
}

/// 参数提取辅助函数 - 从JSON中提取字符串参数
pub fn extract_string_param(body: &Value, param_name: &str, alt_name: Option<&str>) -> Result<String, anyhow::Error> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("参数 {} 缺失或格式错误", param_name))
}

/// 参数提取辅助函数 - 从JSON中提取可选字符串参数
pub fn get_optional_string(body: &Value, param_name: &str, alt_name: Option<&str>) -> Option<String> {
    body.get(param_name)
        .or_else(|| alt_name.and_then(|alt| body.get(alt)))
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            }
        })
}

/// 参数提取辅助函数 - 从JSON中提取i64整数参数
pub fn get_i64_param(body: &Value, param_name: &str, default: i64) -> i64 {
    body.get(param_name)
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
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