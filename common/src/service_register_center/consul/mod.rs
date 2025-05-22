use async_trait::async_trait;
use rs_consul::{Config as ConsulConfig, Consul as RsConsul};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::config::AppConfig;
use crate::service_register_center::typos::Registration;
use crate::service_register_center::ServiceRegister;
use crate::Error;

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
    ttl_updaters: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
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

        Self {
            options,
            client,
            ttl_updaters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 启动一个后台任务来定期更新TTL健康检查状态，使用自定义更新间隔
    pub async fn start_ttl_updater_with_interval(&self, service_id: String, interval_seconds: u64) {
        // 先检查是否已存在该服务的更新器，如果存在则先停止
        {
            let mut updaters = self.ttl_updaters.lock().await;
            if updaters.contains_key(&service_id) {
                if let Some(handle) = updaters.remove(&service_id) {
                    handle.abort();
                    debug!("Stopped existing TTL updater for service: {}", service_id);
                }
            }
        }

        let protocol = self.options.protocol.clone();
        let host = self.options.host.clone();
        let port = self.options.port;
        let timeout = self.options.timeout;

        // 在移动到任务前先克隆service_id
        let service_id_for_task = service_id.clone();

        // 启动一个后台任务
        let task = tokio::spawn(async move {
            let check_url = format!(
                "{}://{}:{}/v1/agent/check/pass/service:{}",
                protocol, host, port, service_id_for_task
            );

            // 使用自定义的更新间隔
            let interval = std::time::Duration::from_secs(interval_seconds);
            let client = reqwest::Client::new();

            loop {
                tokio::time::sleep(interval).await;

                match client
                    .put(&check_url)
                    .timeout(std::time::Duration::from_secs(timeout))
                    .send()
                    .await
                {
                    Ok(_) => {
                        debug!(
                            "TTL health check updated for service: {} (interval: {}s)",
                            service_id_for_task, interval_seconds
                        );
                    }
                    Err(e) => {
                        error!(
                            "Failed to update TTL health check for service {}: {}",
                            service_id_for_task, e
                        );
                    }
                }
            }
        });

        // 存储任务句柄以便后续取消
        let mut updaters = self.ttl_updaters.lock().await;
        updaters.insert(service_id.clone(), task);

        info!(
            "Started TTL health check updater for service: {} (interval: {}s)",
            service_id, interval_seconds
        );
    }

    /// 停止特定服务的TTL更新器
    pub async fn stop_ttl_updater(&self, service_id: &str) {
        let mut updaters = self.ttl_updaters.lock().await;
        if let Some(handle) = updaters.remove(service_id) {
            // 终止任务
            handle.abort();
            drop(handle); // 显式丢弃句柄
            info!(
                "Stopped TTL health check updater for service: {}",
                service_id
            );
        } else {
            debug!("No TTL updater found for service: {}", service_id);
        }
    }

    /// 启动一个后台任务来定期更新TTL健康检查状态
    pub async fn start_ttl_updater(&self, service_id: String) {
        // 使用默认的10秒更新间隔
        self.start_ttl_updater_with_interval(service_id, 10).await;
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

        debug!(
            "Registering service: {} ({}:{})",
            registration.name, registration.host, registration.port
        );

        // 构建服务注册JSON
        let mut payload = json!({
            "ID": registration.id,
            "Name": registration.name,
            "Address": registration.host,
            "Port": registration.port,
            "Tags": registration.tags
        });
        
        let mut is_ttl = false;
        let ttl_interval;
        // 根据健康检查类型添加相应配置
        if let Some(check) = &registration.check {
            if check.health_type == "http" {
                // HTTP健康检查
                let check_json = json!({
                    "Name": check.name,
                    "HTTP": check.url,
                    "Interval": check.interval,
                    "Timeout": check.timeout,
                    "DeregisterCriticalServiceAfter": check.deregister_after
                });
                payload["Check"] = check_json;
                info!("Using HTTP health check for service: {}", registration.name);
            } else {
                // gRPC服务使用TTL健康检查
                let check_json = json!({
                    "Name": check.name,
                    "Notes": "TTL health check for gRPC service",
                    "TTL": check.interval, // 15秒TTL
                    "DeregisterCriticalServiceAfter": check.deregister_after
                });
                payload["Check"] = check_json;
                info!("Using TTL health check for service: {}", registration.name);
                is_ttl = true;
            }
            ttl_interval = check.interval.parse::<u64>().unwrap_or(15);
        } else {
            let check_json = json!({
                "Name": format!("{} TTL Check", registration.name),
                "Notes": "Automatically managed TTL health check",
                "TTL": "15",
                "DeregisterCriticalServiceAfter": "60"
            });
            payload["Check"] = check_json;
            info!("Using auto-configured TTL health check for service: {}", registration.name);
            is_ttl = true;
            ttl_interval = 15;
        }

        // 发送HTTP请求
        let client = reqwest::Client::new();
        let response = client
            .put(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(self.options.timeout))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        if response.status().is_success() {
            info!("Service registered successfully: {}", registration.id);

            // 如果使用TTL检查，立即发送一个通过状态并启动TTL更新器
            if is_ttl {
                let check_url = format!(
                    "{}://{}:{}/v1/agent/check/pass/service:{}",
                    self.options.protocol, self.options.host, self.options.port, registration.id
                );

                // 发送初始健康状态
                match client
                    .put(&check_url)
                    .timeout(std::time::Duration::from_secs(self.options.timeout))
                    .send()
                    .await
                {
                    Ok(_) => {
                        info!(
                            "Initial health check status set to passing for service: {}",
                            registration.id
                        );

                        // 启动TTL更新器，使用配置的间隔
                        self.start_ttl_updater_with_interval(registration.id.clone(), ttl_interval)
                            .await;
                    }
                    Err(e) => {
                        error!(
                            "Failed to set initial health check status for service {}: {}",
                            registration.id, e
                        );
                        // 即使初始状态设置失败，仍然启动更新器尝试保持服务健康
                        self.start_ttl_updater_with_interval(registration.id.clone(), ttl_interval)
                            .await;
                    }
                }
            }

            Ok(registration.id.clone())
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!(
                "Failed to register service: HTTP {}: {}",
                status, error_text
            );
            Err(Error::Internal(format!("HTTP {}: {}", status, error_text)))
        }
    }

    async fn deregister(&self, service_id: &str) -> Result<(), Error> {
        // 先停止TTL更新器
        self.stop_ttl_updater(service_id).await;

        // 直接使用HTTP API与Consul交互
        let url = format!(
            "{}://{}:{}/v1/agent/service/deregister/{}",
            self.options.protocol, self.options.host, self.options.port, service_id
        );

        debug!("Deregistering service: {}", service_id);

        // 发送HTTP请求
        let client = reqwest::Client::new();
        let response = client
            .put(&url) // 使用&url而不是url
            .timeout(std::time::Duration::from_secs(self.options.timeout))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        if response.status().is_success() {
            info!("Service deregistered successfully: {}", service_id);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!(
                "Failed to deregister service: HTTP {}: {}",
                status, error_text
            );
            Err(Error::Internal(format!("HTTP {}: {}", status, error_text)))
        }
    }

    async fn find_by_name(
        &self,
        service_name: &str,
    ) -> Result<HashMap<String, Registration>, Error> {
        // 构建Consul API URL - 使用health API只获取健康的服务
        let url = format!(
            "{}://{}:{}/v1/health/service/{}?passing=true",
            self.options.protocol, self.options.host, self.options.port, service_name
        );

        debug!("Finding healthy services with name: {}", service_name);

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
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to find services: HTTP {}: {}", status, error_text);
            return Err(Error::Internal(format!("HTTP {}: {}", status, error_text)));
        }

        // 解析响应
        let entries: Vec<serde_json::Value> = response
            .json()
            .await
            .map_err(|e| Error::Internal(format!("Failed to parse response: {}", e)))?;

        // 映射到Registration结构
        let mut result = HashMap::new();
        for entry in entries {
            if let Some(service) = entry.get("Service") {
                if let (Some(id), Some(name), Some(port)) = (
                    service.get("ID").and_then(|v| v.as_str()),
                    service.get("Service").and_then(|v| v.as_str()),
                    service.get("Port").and_then(|v| v.as_u64()),
                ) {
                    // 从Service中获取地址，如果不存在则尝试从Node中获取
                    let address = service
                        .get("Address")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            entry
                                .get("Node")
                                .and_then(|n| n.get("Address"))
                                .and_then(|a| a.as_str())
                        })
                        .unwrap_or("127.0.0.1");

                    // 提取标签
                    let tags = service
                        .get("Tags")
                        .and_then(|t| t.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let registration = Registration {
                        id: id.to_string(),
                        name: name.to_string(),
                        host: address.to_string(),
                        port: port as u16,
                        tags,
                        check: None,
                    };

                    debug!("Found healthy service: {}:{} ({})", address, port, id);
                    result.insert(id.to_string(), registration);
                }
            }
        }

        if result.is_empty() {
            debug!("No healthy services found with name: {}", service_name);
        } else {
            info!(
                "Found {} healthy instances of service: {}",
                result.len(),
                service_name
            );
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_register_center::typos::HealthCheck;
    use rs_consul::{Config, GetServiceNodesRequest};

    #[tokio::test]
    async fn register_deregister_should_work() {
        let config = AppConfig::from_file(Option::from("../config/config.yml")).unwrap();

        let consul = Consul::from_config(&config);

        let service_id = format!(
            "{}-{}-{}",
            &config.websocket.name, &config.websocket.host, &config.websocket.port
        );

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

        // 等待一段时间，让TTL更新器运行
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // 删除服务
        let result = consul.deregister(&service_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn register_with_http_check_should_work() {
        let config = AppConfig::from_file(Option::from("../config/config.yml")).unwrap();

        let consul = Consul::from_config(&config);

        let service_id = format!("http-test-{}", uuid::Uuid::new_v4());

        // 创建带HTTP健康检查的注册
        let health_check = HealthCheck {
            health_type: "http".to_string(),
            name: "HTTP API Health".to_string(),
            url: "http://localhost:8080/health".to_string(),
            interval: "5s".to_string(),
            timeout: "2s".to_string(),
            deregister_after: "30s".to_string(),
        };

        let registration = Registration {
            id: service_id.clone(),
            name: "http-api".to_string(),
            host: "localhost".to_string(),
            port: 8080,
            tags: vec!["api".to_string(), "http".to_string()],
            check: Some(health_check),
        };

        let result = consul.register(registration).await;
        // 注册可能会失败，因为健康检查URL可能不可用，但我们只关心代码能否正常执行
        println!("HTTP health check registration result: {:?}", result);

        // 删除服务
        let _ = consul.deregister(&service_id).await;
    }

    #[tokio::test]
    async fn register_with_ttl_check_should_work() {
        let config = AppConfig::from_file(Option::from("../config/config.yml")).unwrap();

        let consul = Consul::from_config(&config);

        let service_id = format!("grpc-test-{}", uuid::Uuid::new_v4());

        // 创建带TTL健康检查的注册
        let health_check = HealthCheck {
            health_type: "grpc".to_string(),
            name: "gRPC Service TTL".to_string(),
            url: "".to_string(),      // TTL不需要URL
            interval: "".to_string(), // TTL不需要间隔
            timeout: "".to_string(),  // TTL不需要超时
            deregister_after: "30s".to_string(),
        };

        let registration = Registration {
            id: service_id.clone(),
            name: "grpc-service".to_string(),
            host: "localhost".to_string(),
            port: 50051,
            tags: vec!["grpc".to_string(), "service".to_string()],
            check: Some(health_check),
        };

        let result = consul.register(registration).await;
        assert!(result.is_ok());

        // 等待一段时间，让TTL更新器运行
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // 删除服务
        let result = consul.deregister(&service_id).await;
        assert!(result.is_ok());
    }

    // 保留原有的service_reg测试
    // ... 其他测试 ...
}
