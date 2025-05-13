use regex::Regex;
use crate::config::routes_config::PathRewrite;
use tracing::debug;
use hyper::http::{self, header::HeaderValue};

/// 应用路径重写规则
pub fn apply_path_rewrite(path: &str, path_prefix: &str, rewrite: &PathRewrite) -> String {
    let mut result = path.to_string();
    
    // 应用前缀替换
    if let Some(replace_prefix) = &rewrite.replace_prefix {
        if path.starts_with(path_prefix) {
            result = format!("{}{}", replace_prefix, &path[path_prefix.len()..]);
            debug!("应用前缀替换: {} -> {}", path, result);
        }
    }
    
    // 应用正则替换
    if let (Some(regex_match), Some(regex_replace)) = (&rewrite.regex_match, &rewrite.regex_replace) {
        if let Ok(re) = Regex::new(regex_match) {
            let replaced = re.replace_all(&result, regex_replace).to_string();
            if replaced != result {
                debug!("应用正则替换: {} -> {}", result, replaced);
                result = replaced;
            }
        }
    }
    
    result
}

/// 提取服务类型
pub fn extract_service_type(path: &str) -> &'static str {
    if path.starts_with("/api/auth") {
        "auth"
    } else if path.starts_with("/api/users") {
        "user"
    } else if path.starts_with("/api/friends") {
        "friend"
    } else if path.starts_with("/api/groups") {
        "group"
    } else {
        "unknown"
    }
}

/// 添加跟踪头
pub fn add_tracing_headers(headers: &mut http::HeaderMap, trace_id: &str, span_id: &str) {
    // 安全地添加trace-id
    if let Ok(value) = HeaderValue::from_str(trace_id) {
        headers.insert("X-Trace-ID", value);
    }
    
    // 安全地添加span-id
    if let Ok(value) = HeaderValue::from_str(span_id) {
        headers.insert("X-Span-ID", value);
    }
}

/// 合并URL
pub fn join_url(base: &str, path: &str) -> String {
    let base_ends_with_slash = base.ends_with('/');
    let path_starts_with_slash = path.starts_with('/');
    
    match (base_ends_with_slash, path_starts_with_slash) {
        (true, true) => format!("{}{}", base, &path[1..]),
        (false, false) => format!("{}/{}", base, path),
        _ => format!("{}{}", base, path),
    }
} 