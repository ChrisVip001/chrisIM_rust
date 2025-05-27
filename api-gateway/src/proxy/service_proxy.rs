use crate::auth::jwt::UserInfo;
use crate::proxy::grpc_client::{GrpcClientFactory, GrpcClientFactoryImpl};
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use common::configs::routes_config::ServiceType;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error};
use common::config::{AppConfig, ConfigLoader};
use common::service_register_center::{service_register_center, ServiceRegister};
use common::Error;

/// 服务代理 - 负责转发请求到后端服务
#[derive(Clone)]
pub struct ServiceProxy {
    // 服务注册中心
    service_register: Arc<dyn ServiceRegister>,
    // 应用配置
    config: Arc<AppConfig>,
    // HTTP 客户端
    http_client: Client,
    // gRPC 客户端工厂
    grpc_client_factory: GrpcClientFactoryImpl,
}

impl ServiceProxy {
    /// 创建新的服务代理
    pub async fn new() -> Self {
        // 加载配置
        let config = ConfigLoader::get_global().expect("全局配置单例未初始化");
        
        // 创建服务注册中心
        let service_register = service_register_center(&config);

        // 创建HTTP客户端
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(100)
            .build()
            .unwrap_or_default();

        // 创建gRPC客户端工厂
        let grpc_client_factory = GrpcClientFactoryImpl::new();

        Self {
            service_register,
            config,
            http_client,
            grpc_client_factory,
        }
    }

    /// 转发请求到后端服务
    pub async fn forward_request(
        &self,
        req: Request<Body>,
        service_type: &ServiceType,
    ) -> Response<Body> {
        // 根据服务类型决定转发方式
        match service_type {
            // 核心业务服务使用gRPC转发
            ServiceType::User | ServiceType::Friend | ServiceType::Group | ServiceType::Common | ServiceType::Chat => {
                self.forward_grpc_request(req, service_type).await
            }
            // HTTP服务或静态服务使用HTTP转发
            ServiceType::Static => {
                self.forward_http_request_by_type(req, service_type).await
            }
        }
    }

    /// 转发gRPC请求
    async fn forward_grpc_request(
        &self,
        req: Request<Body>,
        service_type: &ServiceType,
    ) -> Response<Body> {
        let service_name = self.get_service_name(service_type);
        
        // 获取目标服务地址
        match self.get_service_url(&service_name).await {
            Ok(service_url) => {
                debug!("转发gRPC请求到服务: {}", service_url);
                self.grpc_client_factory.forward_request(req, service_url).await
            }
            Err(e) => {
                error!("无法获取gRPC服务地址: {}", e);
                self.service_unavailable_response(&service_name)
            }
        }
    }

    /// 转发HTTP请求（根据服务类型）
    async fn forward_http_request_by_type(
        &self,
        req: Request<Body>,
        service_type: &ServiceType,
    ) -> Response<Body> {
        let service_name = self.get_service_name(service_type);

        // 获取目标服务地址
        match self.get_service_url(&service_name).await {
            Ok(service_url) => {
                debug!("转发HTTP请求到服务: {}", service_url);
                self.forward_http_request(req, &service_url).await
            }
            Err(e) => {
                error!("无法获取HTTP服务地址: {}", e);
                self.service_unavailable_response(&service_name)
            }
        }
    }
    
    /// 从服务注册中心获取服务URL
    async fn get_service_url(&self, service_name: &str) -> Result<String, Error> {
        // 从服务注册中心获取服务信息
        let services = self.service_register.find_by_name(service_name).await?;
        
        if services.is_empty() {
            return Err(Error::NotFound(format!("服务不可用: {}", service_name)));
        }
        
        // 简单的负载均衡：随机选择一个服务实例
        let service = services.values().next().unwrap();
        
        // 构建服务URL
        let protocol = &self.config.service_center.protocol;
        let url = format!("{}://{}:{}", protocol, service.host, service.port);
        
        Ok(url)
    }

    /// 从服务类型获取服务名称
    fn get_service_name(&self, service_type: &ServiceType) -> String {
        match service_type {
            ServiceType::User => "user".to_string(),
            ServiceType::Friend => "friend".to_string(),
            ServiceType::Group => "group".to_string(),
            ServiceType::Chat => "chat".to_string(),
            ServiceType::Static => "static".to_string(),
            ServiceType::Common => "common".to_string(),
          }
    }

    /// 转发HTTP请求
    async fn forward_http_request(&self, req: Request<Body>, service_url: &str) -> Response<Body> {
        let (parts, body) = req.into_parts();
        let path_query = parts
            .uri
            .path_and_query()
            .map(|v| v.as_str())
            .unwrap_or(parts.uri.path());

        let target_url = format!("{}{}", service_url, path_query);
        debug!("转发HTTP请求: {} -> {}", parts.uri.path(), target_url);

        // 读取请求体
        let body_bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(e) => {
                error!("读取请求体失败: {}", e);
                return self.bad_request_response("无法读取请求体");
            }
        };

        // 构建reqwest请求
        let mut client_req = match parts.method.as_str() {
            "GET" => self.http_client.get(&target_url),
            "POST" => self.http_client.post(&target_url).body(body_bytes.to_vec()),
            "PUT" => self.http_client.put(&target_url).body(body_bytes.to_vec()),
            "DELETE" => self.http_client.delete(&target_url),
            "PATCH" => self.http_client.patch(&target_url).body(body_bytes.to_vec()),
            "HEAD" => self.http_client.head(&target_url),
            "OPTIONS" => self.http_client.request(reqwest::Method::OPTIONS, &target_url),
            _ => {
                error!("不支持的HTTP方法: {}", parts.method);
                return self.method_not_allowed_response();
            }
        };

        // 复制请求头（排除某些不应转发的头）
        for (name, value) in parts.headers.iter() {
            if !should_skip_header(name.as_str()) {
                if let Ok(value_str) = value.to_str() {
                    client_req = client_req.header(name.as_str(), value_str);
                }
            }
        }

        // 从请求扩展获取用户信息，并添加到请求头中
        if let Some(user_info) = parts.extensions.get::<UserInfo>() {
            client_req = client_req.header("X-User-ID", user_info.user_id.to_string());
            client_req = client_req.header("X-Username", &user_info.username);

            // 添加用户租户信息
            client_req = client_req.header("X-Tenant-ID", user_info.tenant_id.to_string());
            client_req = client_req.header("X-Tenant-Name", &user_info.tenant_name);
        }

        // 添加原始路径和方法到请求头
        client_req = client_req.header("X-Original-Path", parts.uri.path());
        client_req = client_req.header("X-Original-Method", parts.method.as_str());

        // 发送请求
        match client_req.send().await {
            Ok(resp) => self.convert_response(resp).await,
            Err(e) => {
                error!("转发请求失败: {}", e);
                self.service_unavailable_response("目标服务")
            }
        }
    }

    /// 转换reqwest响应为axum响应
    async fn convert_response(&self, resp: reqwest::Response) -> Response<Body> {
        let status = resp.status();
        let headers = resp.headers().clone();
        
        match resp.bytes().await {
            Ok(body) => {
                let mut response = Response::builder().status(status);
                
                // 复制响应头
                for (name, value) in headers.iter() {
                    if !should_skip_header(name.as_str()) {
                        response = response.header(name, value);
                    }
                }
                
                response.body(Body::from(body)).unwrap_or_else(|_| {
                    self.internal_server_error_response("构建响应失败")
                })
            }
            Err(e) => {
                error!("读取响应体失败: {}", e);
                self.internal_server_error_response("读取响应失败")
            }
        }
    }

    /// 服务不可用响应
    fn service_unavailable_response(&self, service_name: &str) -> Response<Body> {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "service_unavailable",
                "message": format!("服务暂时不可用: {}", service_name)
            })),
        )
            .into_response()
    }

    /// 错误请求响应
    fn bad_request_response(&self, message: &str) -> Response<Body> {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": "bad_request",
                "message": message
            })),
        )
            .into_response()
    }

    /// 方法不允许响应
    fn method_not_allowed_response(&self) -> Response<Body> {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            axum::Json(serde_json::json!({
                "error": "method_not_allowed",
                "message": "不支持的HTTP方法"
            })),
        )
            .into_response()
    }

    /// 内部服务器错误响应
    fn internal_server_error_response(&self, message: &str) -> Response<Body> {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({
                "error": "internal_server_error",
                "message": message
            })),
        )
            .into_response()
    }
}

/// 检查是否应该跳过某个请求头
fn should_skip_header(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "host" | "connection" | "transfer-encoding" | "content-length"
    )
}
