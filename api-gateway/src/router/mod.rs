use std::sync::Arc;
use axum::Router;
use axum::routing::{get, any};
use axum::http::{StatusCode, Request};
use axum::response::IntoResponse;
use axum::body::Body;
use axum::middleware;
use axum::Json;
use serde_json::json;
use tracing::info;
use crate::config::CONFIG;
use crate::proxy::service_proxy::ServiceProxy;
use crate::auth::middleware::auth_middleware;
use std::collections::HashMap;

/// 路由构建器
pub struct RouterBuilder {
    service_proxy: Arc<ServiceProxy>,
    router: Router,
}

impl RouterBuilder {
    /// 创建新的路由构建器
    pub fn new(service_proxy: Arc<ServiceProxy>) -> Self {
        Self {
            service_proxy,
            router: Router::new(),
        }
    }
    
    /// 构建动态路由
    pub async fn build(mut self) -> anyhow::Result<Router> {
        // 读取配置
        let config = CONFIG.read().await;
        let routes_config = &config.routes;
        
        // 遍历路由配置，添加到路由器中
        for route in &routes_config.routes {
            let path = route.path_prefix.clone();
            let service_type = route.service_type.clone();
            let require_auth = route.require_auth;
            
            // 创建路由处理函数
            let service_proxy = self.service_proxy.clone();
            let handler = any(move |req: Request<Body>| {
                let service_proxy = service_proxy.clone();
                let service_type = service_type.clone();
                async move {
                    // 将请求转发到目标服务
                    service_proxy.forward_request(req, &service_type).await
                }
            });
            
            // 根据是否需要认证添加中间件
            let route_path = path.clone();
            if require_auth {
                info!("添加需要认证的路由: {}", route_path);
                self.router = self.router.route(
                    &route_path,
                    handler.clone().route_layer(middleware::from_fn(auth_middleware))
                );
            } else {
                info!("添加无需认证的路由: {}", route_path);
                self.router = self.router.route(&route_path, handler.clone());
            }
            
            // 处理通配符路径
            let wildcard_path = format!("{}/*path", path);
            if require_auth {
                self.router = self.router.route(
                    &wildcard_path,
                    handler.clone().route_layer(middleware::from_fn(auth_middleware))
                );
            } else {
                self.router = self.router.route(&wildcard_path, handler.clone());
            }
        }
        
        // 添加健康检查和指标端点
        self.router = self.router
            .route("/health", get(health_check))
            .route(&config.metrics_endpoint, get(crate::metrics::get_metrics_handler));
        
        Ok(self.router.with_state(()))
    }
}

/// 健康检查处理函数
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// 路由注册器 - 用于动态更新路由
pub struct RouteRegistry {
    routes: Arc<tokio::sync::RwLock<HashMap<String, Router>>>,
}

impl RouteRegistry {
    /// 创建新的路由注册器
    pub fn new() -> Self {
        Self {
            routes: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }
    
    /// 注册路由
    pub async fn register_route(&self, path: &str, router: Router) {
        let mut routes = self.routes.write().await;
        routes.insert(path.to_string(), router);
        info!("路由已注册: {}", path);
    }
    
    /// 移除路由
    pub async fn remove_route(&self, path: &str) {
        let mut routes = self.routes.write().await;
        routes.remove(path);
        info!("路由已移除: {}", path);
    }
    
    /// 获取路由
    pub async fn get_route(&self, path: &str) -> Option<Router> {
        let routes = self.routes.read().await;
        routes.get(path).cloned()
    }
    
    /// 获取所有路由路径
    pub async fn get_all_routes(&self) -> Vec<String> {
        let routes = self.routes.read().await;
        routes.keys().cloned().collect()
    }
} 