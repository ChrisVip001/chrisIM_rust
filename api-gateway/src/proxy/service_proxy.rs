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
        // 获取目标服务名称
        let service_name = self.get_service_name(service_type);

        // 获取目标服务地址
        match self.get_service_url(&service_name).await {
            Ok(service_url) => {
                debug!("转发请求到服务: {}", service_url);

                // 根据服务类型选择转发方式
                match service_type {
                    ServiceType::HttpService(_) | ServiceType::Static => {
                        // 转发HTTP请求
                        self.forward_http_request(req, &service_url).await
                    }
                    ServiceType::User
                    | ServiceType::Friend
                    | ServiceType::Group
                    | ServiceType::Chat
                    | ServiceType::GrpcService(_) => {
                        // 转发gRPC请求
                        self.forward_grpc_request(req, &service_url).await
                    }
                }
            }
            Err(e) => {
                error!("无法获取服务地址: {}", e);

                // 返回服务不可用错误
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "service_unavailable",
                        "message": format!("服务暂时不可用: {}", service_name)
                    })),
                )
                    .into_response()
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
            ServiceType::HttpService(name) => name.clone(),
            ServiceType::GrpcService(name) => name.clone(),
        }
    }

    /// 转发HTTP请求
    async fn forward_http_request(&self, req: Request<Body>, service_url: &str) -> Response<Body> {
        // 获取路径
        let path = req.uri().path().to_string();
        let path_query = req
            .uri()
            .path_and_query()
            .map(|v| v.as_str())
            .unwrap_or(&path);

        // 简化路由匹配逻辑，直接使用原始路径
        let target_path = path_query.to_string();

        // 构建目标URL
        let target_url = format!("{}{}", service_url, target_path);

        debug!("转发HTTP请求: {} -> {}", path, target_url);

        // 创建新的请求
        let (parts, body) = req.into_parts();

        // 读取请求体
        let body_bytes = axum::body::to_bytes(body, 1024 * 1024 * 10)
            .await
            .unwrap_or_default();

        // 获取Content-Type和Content-Encoding头
        let content_type = parts
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok());
        let content_encoding = parts
            .headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok());

        // 处理请求体，如果是GZIP压缩的JSON则自动解压
        let processed_body = match crate::proxy::utils::process_request_body(
            &body_bytes,
            content_type,
            content_encoding,
        ) {
            Ok(data) => data,
            Err(e) => {
                error!("处理请求体失败: {}", e);
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": "invalid_request_body",
                        "message": format!("处理请求体失败: {}", e)
                    })),
                )
                    .into_response();
            }
        };

        // 创建reqwest请求
        let mut client_req = match parts.method.as_str() {
            "GET" => self.http_client.get(&target_url),
            "POST" => self.http_client.post(&target_url).body(processed_body),
            "PUT" => self.http_client.put(&target_url).body(processed_body),
            "DELETE" => self.http_client.delete(&target_url),
            "PATCH" => self.http_client.patch(&target_url).body(processed_body),
            "HEAD" => self.http_client.head(&target_url),
            "OPTIONS" => self
                .http_client
                .request(reqwest::Method::OPTIONS, &target_url),
            _ => {
                return (
                    StatusCode::METHOD_NOT_ALLOWED,
                    axum::Json(serde_json::json!({
                        "error": "method_not_allowed",
                        "message": format!("不支持的HTTP方法: {}", parts.method)
                    })),
                )
                    .into_response();
            }
        };

        // 转发请求头
        let mut skip_content_encoding = false;
        if let Some(encoding) = content_encoding {
            skip_content_encoding = encoding.to_lowercase().contains("gzip");
        }

        for (name, value) in parts.headers {
            if let Some(name) = name {
                // 忽略一些特定的头
                if name.as_str() == "host" || name.as_str() == "content-length" {
                    continue;
                }

                // 如果已经解压过GZIP数据，不要转发content-encoding头
                if skip_content_encoding && name.as_str() == "content-encoding" {
                    continue;
                }

                if let Ok(value) = value.to_str() {
                    client_req = client_req.header(name.as_str(), value);
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
        client_req = client_req.header("X-Original-Path", path);
        client_req = client_req.header("X-Original-Method", parts.method.as_str());

        // 发送请求
        match client_req.send().await {
            Ok(resp) => {
                // 构建响应
                let mut builder = Response::builder().status(resp.status());

                // 转发响应头
                let headers = builder.headers_mut().unwrap();
                for (name, value) in resp.headers() {
                    headers.insert(name, value.clone());
                }

                // 读取响应体
                let body_bytes = resp.bytes().await.unwrap_or_default();

                // 构建响应
                builder.body(Body::from(body_bytes)).unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("无法构建响应"))
                        .unwrap()
                })
            }
            Err(e) => {
                error!("转发HTTP请求失败: {}", e);

                (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(serde_json::json!({
                        "error": "bad_gateway",
                        "message": format!("无法转发请求到后端服务: {}", e)
                    })),
                )
                    .into_response()
            }
        }
    }

    /// 转发gRPC请求
    async fn forward_grpc_request(&self, req: Request<Body>, service_url: &str) -> Response<Body> {
        // 使用已创建的 GrpcClientFactoryImpl 实例处理 gRPC 请求
        self.grpc_client_factory.forward_request(req, service_url.to_string()).await
    }
}

// 在ServiceProxy结构体实现后添加Clone实现
impl Clone for ServiceProxy {
    fn clone(&self) -> Self {
        Self {
            service_register: self.service_register.clone(),
            config: self.config.clone(),
            http_client: self.http_client.clone(),
            grpc_client_factory: self.grpc_client_factory.clone(),
        }
    }
}
