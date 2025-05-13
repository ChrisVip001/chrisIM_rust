use std::sync::Arc;
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, error, debug, warn};
use reqwest::Client;
use crate::config::CONFIG;
use crate::config::routes_config::ServiceType;
use crate::auth::jwt::UserInfo;
use rand::Rng;
use crate::proxy::grpc_client::GrpcClientFactory;

/// 服务发现接口
pub struct ServiceDiscovery {
    // 服务地址缓存
    services: RwLock<HashMap<String, Vec<String>>>,
    // Consul客户端
    consul_client: Client,
    // Consul URL
    consul_url: String,
}

impl ServiceDiscovery {
    /// 创建新的服务发现实例
    pub fn new(consul_url: &str) -> Self {
        Self {
            services: RwLock::new(HashMap::new()),
            consul_client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            consul_url: consul_url.to_string(),
        }
    }
    
    /// 发现服务地址
    pub async fn discover_service(&self, service_name: &str) -> Result<Vec<String>, String> {
        // 首先尝试从缓存获取
        {
            let services = self.services.read().await;
            if let Some(addresses) = services.get(service_name) {
                if !addresses.is_empty() {
                    return Ok(addresses.clone());
                }
            }
        }
        
        // 缓存中不存在，从Consul获取
        let consul_url = format!("{}/v1/catalog/service/{}", self.consul_url, service_name);
        
        match self.consul_client.get(&consul_url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Vec<serde_json::Value>>().await {
                        Ok(services) => {
                            let mut addresses = Vec::new();
                            
                            for service in services {
                                if let (Some(address), Some(port)) = (
                                    service.get("ServiceAddress").and_then(|a| a.as_str()),
                                    service.get("ServicePort").and_then(|p| p.as_u64()),
                                ) {
                                    // 构建服务地址
                                    let addr = if address.is_empty() {
                                        // 如果ServiceAddress为空，使用Address
                                        if let Some(addr) = service.get("Address").and_then(|a| a.as_str()) {
                                            format!("http://{}:{}", addr, port)
                                        } else {
                                            continue;
                                        }
                                    } else {
                                        format!("http://{}:{}", address, port)
                                    };
                                    
                                    addresses.push(addr);
                                }
                            }
                            
                            if addresses.is_empty() {
                                return Err(format!("无法找到服务: {}", service_name));
                            }
                            
                            // 更新缓存
                            {
                                let mut services = self.services.write().await;
                                services.insert(service_name.to_string(), addresses.clone());
                            }
                            
                            Ok(addresses)
                        },
                        Err(e) => Err(format!("解析服务发现响应失败: {}", e)),
                    }
                } else {
                    Err(format!("服务发现请求失败: HTTP {}", response.status()))
                }
            },
            Err(e) => Err(format!("服务发现请求错误: {}", e)),
        }
    }
    
    /// 获取服务地址（使用简单的负载均衡）
    pub async fn get_service_url(&self, service_name: &str) -> Result<String, String> {
        let addresses = self.discover_service(service_name).await?;
        
        // 简单的轮询负载均衡
        let idx = rand::rng().random_range(0..addresses.len());
        Ok(addresses[idx].clone())
    }
    
    /// 刷新服务缓存
    pub async fn refresh_services(&self) {
        let services = {
            let services = self.services.read().await;
            services.keys().cloned().collect::<Vec<_>>()
        };
        
        for service_name in services {
            match self.discover_service(&service_name).await {
                Ok(_) => debug!("服务 {} 缓存已更新", service_name),
                Err(e) => warn!("刷新服务 {} 缓存失败: {}", service_name, e),
            }
        }
    }
}

/// 服务代理 - 负责转发请求到后端服务
pub struct ServiceProxy {
    // 服务发现
    service_discovery: Arc<ServiceDiscovery>,
    // HTTP 客户端
    http_client: Client,
    // gRPC 客户端工厂
    grpc_clients: RwLock<HashMap<String, Arc<dyn crate::proxy::grpc_client::GrpcClientFactory + Send + Sync>>>,
}

impl ServiceProxy {
    /// 创建新的服务代理
    pub async fn new() -> Self {
        let config = CONFIG.read().await;
        
        // 创建服务发现
        let service_discovery = Arc::new(ServiceDiscovery::new(&config.consul_url));
        
        // 创建HTTP客户端
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(100)
            .build()
            .unwrap_or_default();
        
        Self {
            service_discovery,
            http_client,
            grpc_clients: RwLock::new(HashMap::new()),
        }
    }
    
    /// 转发请求到后端服务
    pub async fn forward_request(&self, req: Request<Body>, service_type: &ServiceType) -> Response<Body> {
        // 获取目标服务名称
        let service_name = self.get_service_name(service_type);
        
        // 获取目标服务地址
        match self.service_discovery.get_service_url(&service_name).await {
            Ok(service_url) => {
                debug!("转发请求到服务: {}", service_url);
                
                // 根据服务类型选择转发方式
                match service_type {
                    ServiceType::HttpService(_) | ServiceType::Auth | ServiceType::User | ServiceType::Friend | ServiceType::Group | ServiceType::Static | ServiceType::Chat => {
                        // 转发HTTP请求
                        self.forward_http_request(req, &service_url).await
                    },
                    ServiceType::GrpcService(_) => {
                        // 转发gRPC请求
                        self.forward_grpc_request(req, &service_url).await
                    },
                }
            },
            Err(e) => {
                error!("无法获取服务地址: {}", e);
                
                // 返回服务不可用错误
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "service_unavailable",
                        "message": format!("服务暂时不可用: {}", service_name)
                    }))
                ).into_response()
            }
        }
    }
    
    /// 从服务类型获取服务名称
    fn get_service_name(&self, service_type: &ServiceType) -> String {
        match service_type {
            ServiceType::Auth => "auth-service".to_string(),
            ServiceType::User => "user-service".to_string(),
            ServiceType::Friend => "friend-service".to_string(),
            ServiceType::Group => "group-service".to_string(),
            ServiceType::Chat => "chat-service".to_string(),
            ServiceType::Static => "static-service".to_string(),
            ServiceType::HttpService(name) => name.clone(),
            ServiceType::GrpcService(name) => name.clone(),
        }
    }
    
    /// 转发HTTP请求
    async fn forward_http_request(&self, req: Request<Body>, service_url: &str) -> Response<Body> {
        // 获取配置
        let config = CONFIG.read().await;
        
        // 获取路径
        let path = req.uri().path().to_string();
        let path_query = req.uri().path_and_query().map(|v| v.as_str()).unwrap_or(&path);
        
        // 查找匹配的路由规则
        let route_rule = config.routes.routes.iter()
            .find(|r| path.starts_with(&r.path_prefix));
        
        // 应用路径重写
        let target_path = if let Some(rule) = route_rule {
            if let Some(rewrite) = &rule.path_rewrite {
                crate::proxy::utils::apply_path_rewrite(path_query, &rule.path_prefix, rewrite)
            } else {
                path_query.to_string()
            }
        } else {
            path_query.to_string()
        };
        
        // 构建目标URL
        let target_url = format!("{}{}", service_url, target_path);
        
        debug!("转发HTTP请求: {} -> {}", path, target_url);
        
        // 创建新的请求
        let (parts, body) = req.into_parts();
        
        // 读取请求体
        let body_bytes = axum::body::to_bytes(body, 1024 * 1024 * 10).await.unwrap_or_default();
        
        // 创建reqwest请求
        let mut client_req = match parts.method.as_str() {
            "GET" => self.http_client.get(&target_url),
            "POST" => self.http_client.post(&target_url).body(body_bytes),
            "PUT" => self.http_client.put(&target_url).body(body_bytes),
            "DELETE" => self.http_client.delete(&target_url),
            "PATCH" => self.http_client.patch(&target_url).body(body_bytes),
            "HEAD" => self.http_client.head(&target_url),
            "OPTIONS" => self.http_client.request(reqwest::Method::OPTIONS, &target_url),
            _ => {
                return (
                    StatusCode::METHOD_NOT_ALLOWED,
                    axum::Json(serde_json::json!({
                        "error": "method_not_allowed",
                        "message": format!("不支持的HTTP方法: {}", parts.method)
                    }))
                ).into_response();
            }
        };
        
        // 转发请求头
        for (name, value) in parts.headers {
            if let Some(name) = name {
                // 忽略一些特定的头
                if name.as_str() == "host" || name.as_str() == "content-length" {
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
            
            // 添加角色信息
            if !user_info.roles.is_empty() {
                client_req = client_req.header(
                    "X-User-Roles",
                    user_info.roles.join(",")
                );
            }
        }
        
        // 添加原始路径和方法到请求头
        client_req = client_req.header("X-Original-Path", path);
        client_req = client_req.header("X-Original-Method", parts.method.as_str());
        
        // 发送请求
        match client_req.send().await {
            Ok(resp) => {
                // 构建响应
                let mut builder = Response::builder()
                    .status(resp.status());
                
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
            },
            Err(e) => {
                error!("转发HTTP请求失败: {}", e);
                
                (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(serde_json::json!({
                        "error": "bad_gateway",
                        "message": format!("无法转发请求到后端服务: {}", e)
                    }))
                ).into_response()
            }
        }
    }
    
    /// 转发gRPC请求
    async fn forward_grpc_request(&self, req: Request<Body>, service_url: &str) -> Response<Body> {
        // 使用GenericGrpcClientFactory处理gRPC请求
        let factory = crate::proxy::grpc_client::GenericGrpcClientFactory::new();
        factory.forward_request(req, service_url.to_string()).await
    }
    
    /// 启动服务刷新任务
    pub fn start_service_refresh(&self) {
        let service_discovery = self.service_discovery.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            
            loop {
                interval.tick().await;
                service_discovery.refresh_services().await;
            }
        });
    }

    /// 添加shutdown方法
    pub async fn shutdown(&self) {
        info!("准备关闭服务代理...");
        // 清理资源或关闭连接的代码
    }
}

// 在ServiceProxy结构体实现后添加Clone实现
impl Clone for ServiceProxy {
    fn clone(&self) -> Self {
        Self {
            service_discovery: self.service_discovery.clone(),
            http_client: self.http_client.clone(),
            grpc_clients: RwLock::new(HashMap::new()),
        }
    }
}