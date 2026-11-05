pub mod net;

use runtime::grpc_client::base::service_names as sn;

/// 路径 → 真实下游服务名（熔断分桶、指标标签共用）
///
/// 返回 `None` 表示这条路径不经过下游代理（网关自己处理的 `/health`、`/metrics`、
/// 登录/刷新/登出、上传签名，以及 404）。
pub fn upstream_service(path: &str) -> Option<&'static str> {
    // 用户 / 好友 / 群组同属模块化单体 relation
    if path_under(path, "/api/users") || path_under(path, "/api/groups") {
        return Some(sn::RELATION);
    }
    if path_under(path, "/api/friends") {
        // 这条由网关自己查 Redis 实现（presence 域），不走上游
        if path == "/api/friends/online-status" {
            return None;
        }
        return Some(sn::RELATION);
    }
    if path_under(path, "/api/chat") {
        return Some(sn::CHAT);
    }
    None
}

/// 判断 `path` 是否落在 `prefix` 之下，按路径段边界匹配
///
/// 只认完整的路径段：`/api/user/loginXXX` 与 `/api/users2` 都不算命中前缀。
/// 白名单、限流、熔断三条规则都以它为准。
pub fn path_under(path: &str, prefix: &str) -> bool {
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::path_under;

    #[test]
    fn respects_segment_boundary() {
        assert!(path_under("/api/user/login", "/api/user/login"));
        assert!(path_under("/api/users/1/profile", "/api/users"));
        assert!(!path_under("/api/user/loginXXX", "/api/user/login"));
        assert!(!path_under("/api/users2", "/api/users"));
        assert!(!path_under("/healthz", "/health"));
    }
}
