pub mod grpc_clients;
pub mod service_proxy;
pub mod services;

use axum::body::Body;
use axum::http::{Method, Request};
use serde_json::{Map, Value};
use runtime::auth::Claims;
// 导出公共接口
pub use service_proxy::ServiceProxy;

use crate::error::GatewayError;

/// 请求体上限，与 `main.rs` 的 `RequestBodyLimitLayer` 保持同一口径
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// 将请求体和 URL 参数合并到一个 Value 中，并提取用户信息
///
/// - 空 body 视为 `{}`（GET 与无参 POST 的常态）；
/// - body 不是合法 JSON 时返回 400；
/// - query 参数做百分号解码，只补充 body 中缺失的键。
async fn extract_request_body(
    req: Request<Body>,
) -> Result<(Method, String, Value, Option<Claims>), GatewayError> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    // 从请求扩展获取用户信息
    let user_info = req.extensions().get::<Claims>().cloned();

    // 提取请求体
    let body_bytes = axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES)
        .await
        .map_err(|e| GatewayError::bad_request(format!("读取请求体失败: {e}")))?;

    let mut body: Value = if body_bytes.is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_slice(&body_bytes)
            .map_err(|e| GatewayError::bad_request(format!("请求体不是合法 JSON: {e}")))?
    };

    // query 参数作为缺省值合并：body 里显式给了的键不覆盖
    if !query.is_empty() {
        if let Some(obj) = body.as_object_mut() {
            for (key, value) in parse_query(&query) {
                obj.entry(key).or_insert(Value::String(value));
            }
        }
    }

    Ok((method, path, body, user_info))
}

/// 解析 query string：`a=1&b=%E4%B8%AD` → [("a","1"), ("b","中")]
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// 最小百分号解码（`+` 视作空格），不引新依赖
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&input[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // 非法转义按字面量处理，不要因为一个 `%` 就让整个请求失败
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_and_plus() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%E4%B8%AD%E6%96%87"), "中文");
        // 非法转义保持原样，不 panic
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn parses_query_pairs() {
        let pairs = parse_query("a=1&b=%E4%B8%AD&flag");
        assert_eq!(pairs[0], ("a".into(), "1".into()));
        assert_eq!(pairs[1], ("b".into(), "中".into()));
        assert_eq!(pairs[2], ("flag".into(), String::new()));
    }
}
