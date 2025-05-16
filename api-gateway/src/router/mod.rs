use crate::auth::middleware::auth_middleware;
use crate::config::CONFIG;
use crate::proxy::service_proxy::ServiceProxy;
use crate::{auth::controller, UserServiceGrpcClient};
use crate::api_doc::api_docs;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{any, get, post};
use axum::Json;
use axum::Router;
use common::grpc_client::GrpcServiceClient;
use serde_json::json;
use std::sync::Arc;
use tracing::info;

/// 路由构建器
pub struct RouterBuilder {
    service_proxy: Arc<ServiceProxy>,
    user_client: Arc<UserServiceGrpcClient>,
    router: Router,
}

impl RouterBuilder {
    /// 创建新的路由构建器
    pub fn new(service_proxy: Arc<ServiceProxy>) -> Self {
        // 创建用户服务客户端
        let service_client = GrpcServiceClient::from_env("user-service");
        let user_client = Arc::new(UserServiceGrpcClient::new(service_client));
        
        Self {
            service_proxy,
            user_client: user_client.clone(),
            router: Router::new().layer(axum::Extension(user_client)),
        }
    }

    /// 构建动态路由
    pub async fn build(mut self) -> anyhow::Result<Router> {
        // 读取配置
        let config = CONFIG.read().await;
        let routes_config = &config.routes;

        // 添加认证相关路由
        self.add_auth_routes();

        // 添加API文档路由
        self.add_api_docs_routes();

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
                let auth_route = any(handler.clone()).layer(middleware::from_fn(auth_middleware));
                self.router = self.router.clone().route(&route_path, auth_route);
            } else {
                info!("添加无需认证的路由: {}", route_path);
                self.router = self.router.clone().route(&route_path, handler.clone());
            }

            // 处理通配符路径
            let wildcard_path = format!("{}/{{*path}}", path);
            if require_auth {
                let auth_wildcard_route =
                    any(handler.clone()).layer(middleware::from_fn(auth_middleware));
                self.router = self.router.clone().route(&wildcard_path, auth_wildcard_route);
            } else {
                self.router = self.router.clone().route(&wildcard_path, handler.clone());
            }
        }

        // 添加健康检查和指标端点
        self.router = self.router.clone().route("/health", get(health_check)).route(
            &config.metrics_endpoint,
            get(crate::metrics::get_metrics_handler),
        );

        Ok(self.router)
    }

    /// 添加认证相关路由
    fn add_auth_routes(&mut self) {
        info!("添加认证相关路由");
        
        // 添加登录路由
        self.router = self.router.clone().route(
            "/api/user/login",
            post(controller::login),
        );

        // 添加令牌刷新路由
        self.router = self.router.clone().route(
            "/api/user/refresh",
            post(controller::refresh_token),
        );
    }

    /// 添加API文档相关路由
    fn add_api_docs_routes(&mut self) {
        info!("添加API文档路由...");

        // 添加API文档路由
        self.router = api_docs::configure_docs(self.router.clone());
        
        info!("API文档路由添加完成");
    }
}

/// 健康检查处理函数
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({
        "status": "ok",
        "service": "api-gateway",
        "version": env!("CARGO_PKG_VERSION"),
        "api_documentation": {
            "swagger_ui": "/swagger-ui",
            "openapi_json": "/api-doc/openapi.json"
        }
    })))
}
