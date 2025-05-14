use crate::config::routes_config::PathRewrite;
use flate2::read::GzDecoder;
use hyper::http::{self, header::HeaderValue};
use regex::Regex;
use std::io::Read;
use tracing::{debug, error};

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
    if let (Some(regex_match), Some(regex_replace)) = (&rewrite.regex_match, &rewrite.regex_replace)
    {
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

/// 处理请求体，根据Content-Type和Content-Encoding自动解压
pub fn process_request_body(
    body: &[u8],
    content_type: Option<&str>,
    content_encoding: Option<&str>,
) -> Result<Vec<u8>, String> {
    // 如果body为空，直接返回
    if body.is_empty() {
        return Ok(Vec::new());
    }

    // 检查是否为GZIP压缩
    let is_gzipped = match content_encoding {
        Some(encoding) => encoding.to_lowercase().contains("gzip"),
        None => false,
    };

    // 检查是否为JSON数据
    let is_json = match content_type {
        Some(content_type) => content_type.to_lowercase().contains("json"),
        None => false,
    };

    // 如果是GZIP压缩的JSON数据，进行解压
    if is_gzipped {
        debug!("检测到GZIP压缩的请求体，开始解压");

        let mut decoder = GzDecoder::new(body);
        let mut decompressed_data = Vec::new();

        match decoder.read_to_end(&mut decompressed_data) {
            Ok(_) => {
                if is_json {
                    // 验证解压后的数据是否为有效的JSON
                    match serde_json::from_slice::<serde_json::Value>(&decompressed_data) {
                        Ok(_) => {
                            debug!(
                                "成功解压GZIP+JSON数据: {} 字节 -> {} 字节",
                                body.len(),
                                decompressed_data.len()
                            );
                            Ok(decompressed_data)
                        }
                        Err(e) => {
                            error!("解压后的数据不是有效的JSON: {}", e);
                            Err(format!("解压后的数据不是有效的JSON: {}", e))
                        }
                    }
                } else {
                    debug!(
                        "成功解压GZIP数据: {} 字节 -> {} 字节",
                        body.len(),
                        decompressed_data.len()
                    );
                    Ok(decompressed_data)
                }
            }
            Err(e) => {
                error!("GZIP解压失败: {}", e);
                Err(format!("GZIP解压失败: {}", e))
            }
        }
    } else {
        // 如果不是GZIP压缩，直接返回原始数据
        Ok(body.to_vec())
    }
}

// 添加单元测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::routes_config::PathRewrite;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_path_rewrite() {
        let path1 = "/api/users/123";
        let prefix1 = "/api";
        let rewrite1 = PathRewrite {
            replace_prefix: Some("/v1".to_string()),
            regex_match: None,
            regex_replace: None,
        };

        assert_eq!(
            apply_path_rewrite(path1, prefix1, &rewrite1),
            "/v1/users/123"
        );

        let path2 = "/api/users/123";
        let prefix2 = "/api/";
        let rewrite2 = PathRewrite {
            replace_prefix: Some("/v1/".to_string()),
            regex_match: None,
            regex_replace: None,
        };

        assert_eq!(
            apply_path_rewrite(path2, prefix2, &rewrite2),
            "/v1/users/123"
        );

        let path3 = "/other/path";
        let prefix3 = "/api";
        let rewrite3 = PathRewrite {
            replace_prefix: Some("/v1".to_string()),
            regex_match: None,
            regex_replace: None,
        };

        assert_eq!(apply_path_rewrite(path3, prefix3, &rewrite3), "/other/path");
    }

    #[test]
    fn test_process_request_body_not_gzipped() {
        let data = b"{ \"hello\": \"world\" }";
        let result = process_request_body(data, Some("application/json"), None).unwrap();

        assert_eq!(result, data);
    }

    #[test]
    fn test_process_request_body_gzipped_json() {
        // 创建GZIP压缩的JSON数据
        let json_data = b"{ \"hello\": \"world\" }";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json_data).unwrap();
        let compressed_data = encoder.finish().unwrap();

        // 解压数据
        let result =
            process_request_body(&compressed_data, Some("application/json"), Some("gzip")).unwrap();

        assert_eq!(result, json_data);
    }
}
