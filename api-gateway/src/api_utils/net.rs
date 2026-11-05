//! 客户端 IP 解析
//!
//! 日志、限流、认证黑名单共用同一套规则：
//!
//! 1. **对端地址（`ConnectInfo`）是权威**，它不可能被客户端伪造；
//! 2. 只有当对端地址出现在 `auth.trusted_proxies` 里时，才从 `X-Forwarded-For`
//!    的右侧往左找第一个「非受信代理」的地址；
//! 3. 默认 `trusted_proxies` 为空 = 完全忽略 XFF，安全优先。

use axum::{extract::ConnectInfo, http::Request};
use std::net::{IpAddr, SocketAddr};

/// 直连对端地址
pub fn peer_ip<B>(req: &Request<B>) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}

/// 解析真实客户端 IP
pub fn resolve_client_ip<B>(req: &Request<B>, trusted_proxies: &[String]) -> Option<IpAddr> {
    let peer = peer_ip(req)?;

    if !is_trusted(peer, trusted_proxies) {
        return Some(peer);
    }

    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())?;

    // 从右往左：最右侧是紧邻的代理，越往左越接近真实客户端
    forwarded
        .split(',')
        .rev()
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .find(|ip| !is_trusted(*ip, trusted_proxies))
        .or(Some(peer))
}

/// 解析成字符串（用于日志/限流键），解析不出来给 `"unknown"`
pub fn resolve_client_ip_string<B>(req: &Request<B>, trusted_proxies: &[String]) -> String {
    resolve_client_ip(req, trusted_proxies)
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn is_trusted(ip: IpAddr, trusted_proxies: &[String]) -> bool {
    trusted_proxies
        .iter()
        .any(|t| t.parse::<IpAddr>().map(|t| t == ip).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn req(peer: &str, xff: Option<&str>) -> Request<Body> {
        let addr: SocketAddr = format!("{peer}:1234").parse().unwrap();
        let mut builder = Request::builder().uri("/api/chat/send");
        if let Some(xff) = xff {
            builder = builder.header("x-forwarded-for", xff);
        }
        let mut req = builder.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        req
    }

    #[test]
    fn ignores_xff_when_peer_is_not_trusted() {
        // 默认配置（trusted_proxies 为空）：XFF 一律不信
        let r = req("203.0.113.7", Some("1.2.3.4"));
        assert_eq!(resolve_client_ip(&r, &[]).unwrap().to_string(), "203.0.113.7");
    }

    #[test]
    fn uses_xff_only_behind_trusted_proxy() {
        let r = req("127.0.0.1", Some("1.2.3.4, 10.0.0.1"));
        let trusted = vec!["127.0.0.1".to_string(), "10.0.0.1".to_string()];
        // 10.0.0.1 也是受信代理 → 取更左侧的 1.2.3.4
        assert_eq!(resolve_client_ip(&r, &trusted).unwrap().to_string(), "1.2.3.4");
    }

    #[test]
    fn falls_back_to_peer_when_xff_is_garbage() {
        let r = req("127.0.0.1", Some("not-an-ip"));
        let trusted = vec!["127.0.0.1".to_string()];
        assert_eq!(resolve_client_ip(&r, &trusted).unwrap().to_string(), "127.0.0.1");
    }

    #[test]
    fn unknown_without_connect_info() {
        let r = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert!(resolve_client_ip(&r, &[]).is_none());
        assert_eq!(resolve_client_ip_string(&r, &[]), "unknown");
    }
}
