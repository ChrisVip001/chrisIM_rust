use std::collections::HashMap;
use async_trait::async_trait;
use rs_consul::types::*;
use rs_consul::{Config as ConsulConfig, Consul as RsConsul};
use tracing::{debug, error, info};
use uuid;
use serde_json::json;

use crate::utils::get_host_name;
use crate::service_register_center::typos::Registration;
use crate::config::AppConfig;
use crate::Error;
use crate::service_register_center::ServiceRegister;

/// Consul client configuration options
#[derive(Debug, Clone)]
pub struct ConsulOptions {
    pub host: String,
    pub port: u16,
    pub protocol: String,
    pub timeout: u64,
}

impl ConsulOptions {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            host: config.service_center.host.clone(),
            port: config.service_center.port,
            timeout: config.service_center.timeout,
            protocol: config.service_center.protocol.clone(),
        }
    }
}

/// Consul service registry implementation
#[derive(Debug)]
pub struct Consul {
    pub options: ConsulOptions,
    pub client: RsConsul,
}

impl Consul {
    /// Create a new Consul client from application config
    pub fn from_config(config: &AppConfig) -> Self {
        let options = ConsulOptions::from_config(config);

        let consul_url = format!("{}://{}:{}", options.protocol, options.host, options.port);

        let consul_config = ConsulConfig {
            address: consul_url,
            token: None,
            ..Default::default()
        };

        let client = RsConsul::new(consul_config);

        Self { options, client }
    }
}

#[async_trait]
impl ServiceRegister for Consul {
    async fn register(&self, registration: Registration) -> Result<String, Error> {
        // 直接使用HTTP API与Consul交互
        let url = format!(
            "{}://{}:{}/v1/agent/service/register",
            self.options.protocol, self.options.host, self.options.port
        );
        
        debug!("Registering service: {} ({}:{})", registration.name, registration.host, registration.port);
        
        // 构建服务注册JSON，包括健康检查
        let mut payload = json!({
            "ID": registration.id,
            "Name": registration.name,
            "Address": registration.host,
            "Port": registration.port,
            "Tags": registration.tags
        });
        
        // 如果提供了健康检查配置，则添加到payload中
        if let Some(check) = &registration.check {
            let check_json = json!({
                "Name": check.name,
                "HTTP": check.url,
                "Interval": check.interval,
                "Timeout": check.timeout,
                "DeregisterCriticalServiceAfter": check.deregister_after
            });
            
            payload["Check"] = check_json;
        } else {
            // 添加一个默认的TCP健康检查
            let check_json = json!({
                "Name": format!("{} TCP Check", registration.name),
                "TCP": format!("{}:{}", registration.host, registration.port),
                "Interval": "10s",
                "Timeout": "5s",
                "DeregisterCriticalServiceAfter": "1m"
            });
            
            payload["Check"] = check_json;
        }
        
        // 发送HTTP请求
        let client = reqwest::Client::new();
        let response = client
            .put(url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(self.options.timeout))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;
            
        if response.status().is_success() {
            info!("Service registered successfully: {}", registration.id);
            Ok(registration.id.clone())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to register service: HTTP {}: {}", status, error_text);
            Err(Error::Internal(format!("HTTP {}: {}", status, error_text)))
        }
    }

    async fn deregister(&self, service_id: &str) -> Result<(), Error> {
        // 直接使用HTTP API与Consul交互
        let url = format!(
            "{}://{}:{}/v1/agent/service/deregister/{}",
            self.options.protocol, self.options.host, self.options.port, service_id
        );
        
        debug!("Deregistering service: {}", service_id);
        
        // 发送HTTP请求
        let client = reqwest::Client::new();
        let response = client
            .put(url)
            .timeout(std::time::Duration::from_secs(self.options.timeout))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;
            
        if response.status().is_success() {
            info!("Service deregistered successfully: {}", service_id);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to deregister service: HTTP {}: {}", status, error_text);
            Err(Error::Internal(format!("HTTP {}: {}", status, error_text)))
        }
    }

    async fn find_by_name(&self, service_name: &str) -> Result<HashMap<String, Registration>, Error> {
        // 构建Consul API URL
        let url = format!(
            "{}://{}:{}/v1/catalog/service/{}",
            self.options.protocol, self.options.host, self.options.port, service_name
        );
        
        debug!("Finding services with name: {}", service_name);
        
        // 发送HTTP请求
        let client = reqwest::Client::new();
        let response = client
            .get(url)
            .timeout(std::time::Duration::from_secs(self.options.timeout))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;
        
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to find services: HTTP {}: {}", status, error_text);
            return Err(Error::Internal(format!("HTTP {}: {}", status, error_text)));
        }
        
        // 解析响应
        let services: Vec<serde_json::Value> = response.json().await
            .map_err(|e| Error::Internal(format!("Failed to parse response: {}", e)))?;
        
        // 映射到Registration结构
        let mut result = HashMap::new();
        for service in services {
            if let (Some(id), Some(name), Some(address), Some(port)) = (
                service.get("ServiceID").and_then(|v| v.as_str()),
                service.get("ServiceName").and_then(|v| v.as_str()),
                service.get("ServiceAddress").and_then(|v| v.as_str()),
                service.get("ServicePort").and_then(|v| v.as_u64()),
            ) {
                // 提取标签
                let tags = service.get("ServiceTags")
                    .and_then(|t| t.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                
                let registration = Registration {
                    id: id.to_string(),
                    name: name.to_string(),
                    host: address.to_string(),
                    port: port as u16,
                    tags,
                    check: None,
                };
                
                result.insert(id.to_string(), registration);
            }
        }
        
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use rs_consul::Config;
    use super::*;

    #[tokio::test]
    async fn register_deregister_should_work() {
        let config = AppConfig::from_file(Option::from("../config/config.yml")).unwrap();
        
        let consul = Consul::from_config(&config);
        
        let service_id = format!("{}-{}-{}", &config.websocket.name, &config.websocket.host, &config.websocket.port);
        
        let registration = Registration {
            id: service_id.clone(),
            name: config.websocket.name.clone(),
            host: config.websocket.host.clone(),
            port: config.websocket.port,
            tags: config.websocket.tags.clone(),
            check: None,
        };
        let result = consul.register(registration).await;
        assert!(result.is_ok());
        
        
        // delete it
        let result = consul.deregister(&service_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn service_reg() {
        // 初始化 Consul 客户端
        let config = Config::from_env();
        let client = rs_consul::Consul::new(config);
        // 服务名
        let service_name = "user-service";
        // 创建查询请求
        let request = GetServiceNodesRequest {
            service: service_name,
            near: None,
            passing: false,
            filter: None,
        };

        // 查询健康服务实例（包含 IP 和端口）
        let response = client
            .get_service_nodes(request,  None)
            .await;

        // 遍历服务实例
        for entry in response.unwrap().response {
            let service = entry.service;
            println!(
                "Discovered instance: name={}, address={}, port={}",
                service.service, service.address, service.port
            );
        }
    }
}
