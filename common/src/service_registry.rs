use std::time::Duration;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use reqwest::Client;
use tracing::info;
use std::sync::{Arc, RwLock};

/// Consul服务信息结构
#[derive(Debug, Serialize, Deserialize)]
struct ConsulService {
    #[serde(rename = "ServiceID")]
    service_id: String,
    #[serde(rename = "ServiceName")]
    service_name: String,
    #[serde(rename = "ServiceAddress")]
    service_address: String,
    #[serde(rename = "ServicePort")]
    service_port: u32,
}


// Consul服务发现响应
#[derive(Debug, Serialize, Deserialize)]
struct ConsulServicesResponse(Vec<ConsulService>);

/// 服务注册管理器
#[derive(Clone, Debug)]
pub struct ServiceRegistry {
    http_client: Client,
    consul_url: String,
    service_id: Arc<RwLock<Option<String>>>,
}

impl ServiceRegistry {
    /// 创建新的服务注册管理器
    pub fn new(consul_url: &str) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| Client::new());
            
        Self {
            http_client,
            consul_url: consul_url.to_string(),
            service_id: Arc::new(RwLock::new(None)),
        }
    }
    
    /// 从环境变量创建服务注册管理器
    pub fn from_env() -> Self {
        let consul_url = std::env::var("CONSUL_URL")
            .unwrap_or_else(|_| "http://localhost:8500".to_string());
        Self::new(&consul_url)
    }
    
    /// 注册服务到Consul
    pub async fn register_service(
        &self,
        service_name: &str,
        host: &str,
        port: u32,
        tags: Vec<String>,
        health_check_path: &str,
        health_check_interval: &str,
    ) -> Result<String> {
        // 生成唯一服务ID
        let service_id = format!("{}-{}-{}", service_name, host, port);
        
        // 构建注册请求体
        let register_payload = serde_json::json!({
            "ID": service_id,
            "Name": service_name,
            "Tags": tags,
            "Address": host,
            "Port": port,
            "Check": {
                "HTTP": format!("http://{}:{}{}", host, port, health_check_path),
                "Interval": health_check_interval,
                "Timeout": "5s",
                "DeregisterCriticalServiceAfter": "30s",
            }
        });
        
        let url = format!("{}/v1/agent/service/register", self.consul_url);
        
        info!("注册服务 {} 到 Consul: {}", service_name, url);
        
        let response = self.http_client.put(&url)
            .json(&register_payload)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("服务注册失败: 状态码 {}, 消息: {}", status, body));
        }
        
        info!("服务 {} 已成功注册到Consul, 服务ID: {}", service_name, service_id);
        
        // 使用RwLock更新service_id
        if let Ok(mut id) = self.service_id.write() {
            *id = Some(service_id.clone());
        }
        
        Ok(service_id)
    }
    
    /// 从Consul注销服务
    pub async fn deregister_service(&self) -> Result<()> {
        let service_id = match self.service_id.read() {
            Ok(id) => match &*id {
                Some(id) => id.clone(),
                None => return Err(anyhow::anyhow!("没有已注册的服务ID")),
            },
            Err(_) => return Err(anyhow::anyhow!("获取服务ID失败")),
        };
        
        let url = format!("{}/v1/agent/service/deregister/{}", self.consul_url, service_id);
        
        info!("从Consul注销服务: {}", service_id);
        
        let response = self.http_client.put(&url)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("服务注销失败: 状态码 {}, 消息: {}", status, body));
        }
        
        info!("服务 {} 已从Consul注销", service_id);
        Ok(())
    }
    
    /// 发现服务实例
    pub async fn discover_service(&self, service_name: &str) -> Result<Vec<String>> {
        let url = format!("{}/v1/health/service/{}", self.consul_url, service_name);
        
        info!("从Consul查询服务: {}", service_name);
        
        let response = self.http_client.get(&url)
            .query(&[("passing", "true")]) // 只获取健康的服务
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Consul API请求失败: {}", response.status()));
        }
        
        let services: ConsulServicesResponse = response.json().await?;
        
        let service_urls = services.0.into_iter()
            .map(|svc| {
                let host = if svc.service_address.is_empty() {
                    "127.0.0.1".to_string()
                } else {
                    svc.service_address
                };
                format!("http://{}:{}", host, svc.service_port)
            })
            .collect();
        
        Ok(service_urls)
    }
} 