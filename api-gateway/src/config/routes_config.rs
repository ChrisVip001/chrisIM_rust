use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// 路由配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutesConfig {
    /// 路由规则列表
    pub routes: Vec<RouteRule>,
}

/// 路由规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    /// 路由规则ID
    pub id: String,
    /// 路由规则名称
    pub name: String,
    /// 请求路径前缀
    pub path_prefix: String,
    /// 目标服务类型
    pub service_type: ServiceType,
    /// 是否需要认证
    #[serde(default)]
    pub require_auth: bool,
    /// 请求方法限制（如为空则表示全部允许）
    #[serde(default)]
    pub methods: Vec<String>,
    /// 请求头重写规则
    #[serde(default)]
    pub rewrite_headers: HashMap<String, String>,
    /// 路径重写规则
    pub path_rewrite: Option<PathRewrite>,
}

/// 目标服务类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ServiceType {
    /// 认证服务
    Auth,
    /// 用户服务
    User,
    /// 好友服务
    Friend,
    /// 群组服务
    Group,
    /// 聊天服务
    Chat,
    /// 静态资源服务
    Static,
    /// 自定义HTTP服务
    HttpService(String),
    /// 自定义gRPC服务
    GrpcService(String),
}

/// 路径重写规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRewrite {
    /// 路径前缀替换
    pub replace_prefix: Option<String>,
    /// 正则表达式替换
    pub regex_match: Option<String>,
    pub regex_replace: Option<String>,
}

impl Default for RoutesConfig {
    fn default() -> Self {
        Self {
            routes: vec![
                // 默认认证服务路由
                RouteRule {
                    id: "auth-service".to_string(),
                    name: "认证服务".to_string(),
                    path_prefix: "/api/auth".to_string(),
                    service_type: ServiceType::Auth,
                    require_auth: false,
                    methods: vec![],
                    rewrite_headers: HashMap::new(),
                    path_rewrite: Some(PathRewrite {
                        replace_prefix: Some("/".to_string()),
                        regex_match: None,
                        regex_replace: None,
                    }),
                },
                // 默认用户服务路由
                RouteRule {
                    id: "user-service".to_string(),
                    name: "用户服务".to_string(),
                    path_prefix: "/api/users".to_string(),
                    service_type: ServiceType::User,
                    require_auth: true,
                    methods: vec![],
                    rewrite_headers: HashMap::new(),
                    path_rewrite: None,
                },
                // 默认好友服务路由
                RouteRule {
                    id: "friend-service".to_string(),
                    name: "好友服务".to_string(),
                    path_prefix: "/api/friends".to_string(),
                    service_type: ServiceType::Friend,
                    require_auth: true,
                    methods: vec![],
                    rewrite_headers: HashMap::new(),
                    path_rewrite: None,
                },
                // 默认群组服务路由
                RouteRule {
                    id: "group-service".to_string(),
                    name: "群组服务".to_string(),
                    path_prefix: "/api/groups".to_string(),
                    service_type: ServiceType::Group,
                    require_auth: true,
                    methods: vec![],
                    rewrite_headers: HashMap::new(),
                    path_rewrite: None,
                },
                // 默认聊天服务路由
                RouteRule {
                    id: "chat-service".to_string(),
                    name: "聊天服务".to_string(),
                    path_prefix: "/api/chat".to_string(),
                    service_type: ServiceType::Chat,
                    require_auth: true,
                    methods: vec![],
                    rewrite_headers: HashMap::new(),
                    path_rewrite: None,
                },
            ],
        }
    }
} 