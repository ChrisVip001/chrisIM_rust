pub mod http_client;
pub mod service_proxy;
pub mod services;

use axum::body::Body;
use axum::http::{Method, Request};
use serde_json::Value;
use common::auth::Claims;
// 导出公共接口
pub use service_proxy::ServiceProxy;


/// 将请求体和URL参数合并到一个Value中，并提取用户信息
async fn extract_request_body(req: Request<Body>) -> Result<(Method, String, Value, Option<Claims>), anyhow::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());
    // 从请求扩展获取用户信息
    let user_info = req.extensions().get::<Claims>().cloned();

    // 提取请求体
    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|e| anyhow::anyhow!("读取请求体失败: {}", e))?;

    // 解析JSON请求体或URL参数
    let body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(json) => json,
        Err(_) => {
            // 尝试从URL参数获取
            let mut map = serde_json::map::Map::new();
            if let Some(query_str) = query {
                for param in query_str.split('&') {
                    if let Some((key, value)) = param.split_once('=') {
                        map.insert(key.to_string(), Value::String(value.to_string()));
                    }
                }
            }
            Value::Object(map)
        }
    };

    Ok((method, path, body, user_info))
}