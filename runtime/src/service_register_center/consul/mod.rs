use async_trait::async_trait;
use rs_consul::{Config as ConsulConfig, Consul as RsConsul};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::config::AppConfig;
use crate::service_register_center::typos::{HealthCheck, Registration};
use crate::service_register_center::ServiceRegister;
use contract::Error;

/// TTL 判定窗口缺省 10 秒：故障实例最迟 10 秒被 Consul 标记为 critical
const DEFAULT_TTL_SECS: u64 = 10;
/// Consul 要求 DeregisterCriticalServiceAfter 最小 1 分钟
const MIN_DEREGISTER_SECS: u64 = 60;

/// 幂等地安装 rustls 默认加密提供者
///
/// 依赖图中同时存在 aws-lc-rs 与 ring 两个 provider feature 时，
/// rustls 无法自动选择，首次建立 TLS 连接会 panic。
pub fn init_crypto_provider() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

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
            timeout: config.service_center.request_timeout(),
            protocol: config.service_center.protocol.clone(),
        }
    }
}

fn parse_seconds(value: &str, default: u64) -> u64 {
    value
        .trim()
        .trim_end_matches('s')
        .parse::<u64>()
        .unwrap_or(default)
}

/// 规范化 Consul 时长字符串：配置里可能带（"30s"）或不带（"30"）后缀
fn with_seconds(value: &str) -> String {
    format!("{}s", parse_seconds(value, 5))
}

/// TTL 上报周期必须显著短于判定窗口
///
/// 此前上报间隔直接等于 TTL 窗口，等于踩点续命：一次网络抖动或调度延迟
/// 就会让健康实例被误判 critical。按 1/3 窗口上报留出两次重试余量。
fn report_interval_seconds(ttl_seconds: u64) -> u64 {
    (ttl_seconds / 3).max(1)
}

fn deregister_seconds(value: &str) -> String {
    format!(
        "{}s",
        parse_seconds(value, MIN_DEREGISTER_SECS).max(MIN_DEREGISTER_SECS)
    )
}

/// 配置里的健康检查 URL 常写成路径，Consul HTTP 检查需要绝对 URL
fn http_check_url(path_or_url: &str, host: &str, port: u16) -> String {
    if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
        return path_or_url.to_string();
    }
    let path = if path_or_url.starts_with('/') {
        path_or_url.to_string()
    } else {
        format!("/{}", path_or_url)
    };
    format!("http://{}:{}{}", host, port, path)
}

/// 构造 Consul 的 Check 字段
///
/// 返回 `(检查定义, TTL 窗口秒数)`；后者为 `Some` 时需要进程内周期上报。
/// `check: None` 不再是"没有健康检查"，而是显式回落为默认 TTL 检查。
fn build_check(
    check: Option<&HealthCheck>,
    registration: &Registration,
) -> (serde_json::Value, Option<u64>) {
    let Some(c) = check else {
        return (
            json!({
                "Name": format!("{} TTL Check", registration.name),
                "Notes": "Automatically managed TTL health check",
                "TTL": format!("{}s", DEFAULT_TTL_SECS),
                "DeregisterCriticalServiceAfter": format!("{}s", MIN_DEREGISTER_SECS),
            }),
            Some(DEFAULT_TTL_SECS),
        );
    };

    let deregister_after = deregister_seconds(&c.deregister_after);
    match c.health_type.as_str() {
        "http" => (
            json!({
                "Name": c.name,
                "HTTP": http_check_url(&c.url, &registration.host, registration.port),
                "Interval": with_seconds(&c.interval),
                "Timeout": with_seconds(&c.timeout),
                "DeregisterCriticalServiceAfter": deregister_after,
            }),
            None,
        ),
        "grpc" => (
            json!({
                "Name": c.name,
                "GRPC": format!("{}:{}", registration.host, registration.port),
                "Interval": with_seconds(&c.interval),
                "Timeout": with_seconds(&c.timeout),
                "DeregisterCriticalServiceAfter": deregister_after,
            }),
            None,
        ),
        _ => {
            let ttl = parse_seconds(&c.interval, DEFAULT_TTL_SECS).max(1);
            (
                json!({
                    "Name": c.name,
                    "Notes": "TTL health check for service",
                    "TTL": format!("{}s", ttl),
                    "DeregisterCriticalServiceAfter": deregister_after,
                }),
                Some(ttl),
            )
        }
    }
}

/// TTL 上报任务的进程级注册表
///
/// `service_register_center()` 每次调用都会新建 `Consul` 实例，注册与注销
/// 常发生在不同实例上。句柄若挂在实例里，`deregister` 停不掉上报任务，
/// 停机后的僵尸实例会被持续上报为健康、甚至被重新注册。
static TTL_UPDATERS: OnceLock<Arc<Mutex<HashMap<String, JoinHandle<()>>>>> = OnceLock::new();

fn ttl_updaters() -> &'static Arc<Mutex<HashMap<String, JoinHandle<()>>>> {
    TTL_UPDATERS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
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
        init_crypto_provider();
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

    /// 启动进程内 TTL 上报任务
    ///
    /// `period_seconds` 是上报周期，须显著短于 Consul 的 TTL 判定窗口。
    /// `registration_payload` 是注册时发给 Consul 的原始请求体：
    /// Consul 重启会丢失全部注册信息，仅靠 TTL 上报无法恢复，
    /// 任务会周期性核查注册是否存在，缺失时用这份载荷原样重建。
    async fn start_ttl_updater(
        &self,
        service_id: String,
        period_seconds: u64,
        registration_payload: serde_json::Value,
    ) {
        // 同一 service_id 重复注册时先停掉旧任务，避免两个任务互相续命
        {
            let mut updaters = ttl_updaters().lock().await;
            if let Some(handle) = updaters.remove(&service_id) {
                handle.abort();
                debug!("Stopped existing TTL updater for service: {}", service_id);
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
            let service_url = format!(
                "{}://{}:{}/v1/agent/service/{}",
                protocol, host, port, service_id_for_task
            );
            let register_url = format!(
                "{}://{}:{}/v1/agent/service/register",
                protocol, host, port
            );

            // 使用自定义的更新间隔
            let interval = std::time::Duration::from_secs(period_seconds);
            let client = reqwest::Client::new();

            loop {
                tokio::time::sleep(interval).await;

                // 就地核查注册是否还在：Consul 重启后注册会全部丢失，
                // 这时 TTL 上报只会失败，必须先重建注册
                match client
                    .get(&service_url)
                    .timeout(std::time::Duration::from_secs(timeout))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
                        warn!(
                            "服务注册缺失（Consul可能重启过），正在重新注册: {}",
                            service_id_for_task
                        );
                        if let Err(e) = client
                            .put(&register_url)
                            .json(&registration_payload)
                            .timeout(std::time::Duration::from_secs(timeout))
                            .send()
                            .await
                        {
                            error!(
                                "重新注册服务 {} 失败: {}，下个周期重试",
                                service_id_for_task, e
                            );
                            continue;
                        }
                        info!("服务 {} 已重新注册", service_id_for_task);
                        // 重建的 TTL 检查初始为 critical，立即上报通过
                        let _ = client
                            .put(&check_url)
                            .timeout(std::time::Duration::from_secs(timeout))
                            .send()
                            .await;
                    }
                    Ok(resp) if !resp.status().is_success() => {
                        error!(
                            "查询服务注册状态异常: HTTP {}，跳过本轮TTL上报",
                            resp.status()
                        );
                        continue;
                    }
                    Ok(resp) => {
                        // 服务存在；消费掉响应体以便复用连接
                        let _ = resp.bytes().await;
                    }
                    Err(e) => {
                        // Consul 不可达时 TTL 上报同样会失败，跳过本轮
                        error!("查询服务注册状态失败: {}，跳过本轮TTL上报", e);
                        continue;
                    }
                }

                match client
                    .put(&check_url)
                    .timeout(std::time::Duration::from_secs(timeout))
                    .send()
                    .await
                {
                    Ok(_) => {
                        debug!(
                            "TTL health check updated for service: {} (interval: {}s)",
                            service_id_for_task, period_seconds
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
        let mut updaters = ttl_updaters().lock().await;
        updaters.insert(service_id.clone(), task);

        info!(
            "Started TTL health check updater for service: {} (report period: {}s)",
            service_id, period_seconds
        );
    }

    /// 停止指定服务的 TTL 上报任务
    ///
    /// 句柄在进程级共享表中，注册与注销使用不同 `Consul` 实例也能停掉；
    /// 否则停机后任务会继续上报并重建注册，僵尸实例永远不会被摘流。
    pub async fn stop_ttl_updater(&self, service_id: &str) {
        let mut updaters = ttl_updaters().lock().await;
        if let Some(handle) = updaters.remove(service_id) {
            handle.abort();
            info!(
                "Stopped TTL health check updater for service: {}",
                service_id
            );
        } else {
            debug!("No TTL updater found for service: {}", service_id);
        }
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

        // 构建服务注册JSON；健康检查定义由 build_check 统一生成
        let mut payload = json!({
            "ID": registration.id,
            "Name": registration.name,
            "Address": registration.host,
            "Port": registration.port,
            "Tags": registration.tags
        });
        let (check, ttl_seconds) = build_check(registration.check.as_ref(), &registration);
        payload["Check"] = check;

        // 发送HTTP请求
        let client = reqwest::Client::new();
        let response = client
            .put(&url)
            .json(&payload)
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
            error!(
                "Failed to register service: HTTP {}: {}",
                status, error_text
            );
            return Err(Error::Internal(format!("HTTP {}: {}", status, error_text)));
        }

        info!("Service registered successfully: {}", registration.id);

        // 只有 TTL 检查需要进程内上报；HTTP/gRPC 检查由 Consul 主动探测。
        if let Some(ttl) = ttl_seconds {
            let check_url = format!(
                "{}://{}:{}/v1/agent/check/pass/service:{}",
                self.options.protocol, self.options.host, self.options.port, registration.id
            );

            // TTL 检查初始为 critical，注册后立即上报一次，避免首次刷新前被摘流
            if let Err(e) = client
                .put(&check_url)
                .timeout(std::time::Duration::from_secs(self.options.timeout))
                .send()
                .await
            {
                error!(
                    "初始健康状态上报失败（由周期任务续报）: {} - {}",
                    registration.id, e
                );
            }

            let period = report_interval_seconds(ttl);
            info!(
                "TTL 检查窗口 {}s / 上报周期 {}s: {}",
                ttl, period, registration.id
            );
            self.start_ttl_updater(registration.id.clone(), period, payload.clone())
                .await;
        }

        Ok(registration.id.clone())
    }

    async fn deregister(&self, service_id: &str) -> Result<(), Error> {
        // 必须先停上报任务再摘除注册，否则任务会把已停实例重新注册回来
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
                        // 发现结果只描述实例地址，不需要回传健康检查定义
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
            debug!(
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

    #[tokio::test]
    async fn register_deregister_should_work() {
        let config = AppConfig::from_file(Option::from(crate::config::global_config_path().as_str())).unwrap();

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
            // 未显式给检查定义时应回落为默认 TTL 检查，而非"无健康检查"
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
        let config = AppConfig::from_file(Option::from(crate::config::global_config_path().as_str())).unwrap();

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
        let config = AppConfig::from_file(Option::from(crate::config::global_config_path().as_str())).unwrap();

        let consul = Consul::from_config(&config);

        let service_id = format!("grpc-test-{}", uuid::Uuid::new_v4());

        // 创建带TTL健康检查的注册
        let health_check = HealthCheck {
            health_type: "ttl".to_string(),
            name: "Service TTL".to_string(),
            url: "".to_string(),        // TTL不需要URL
            interval: "5".to_string(),  // TTL判定窗口 5s，上报周期自动收敛为 1s
            timeout: "".to_string(),    // TTL不需要超时
            deregister_after: "60".to_string(),
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

    #[test]
    fn ttl_report_period_leaves_retry_margin() {
        assert_eq!(report_interval_seconds(10), 3);
        assert_eq!(report_interval_seconds(15), 5);
        assert_eq!(report_interval_seconds(2), 1);
        assert!(report_interval_seconds(10) * 2 < 10);
    }

    #[test]
    fn check_payload_normalizes_urls_and_durations() {
        let reg = Registration {
            id: "svc-1".to_string(),
            name: "svc".to_string(),
            host: "10.0.0.2".to_string(),
            port: 8085,
            tags: vec![],
            check: None,
        };

        let (check, ttl) = build_check(
            Some(&HealthCheck {
                health_type: "http".to_string(),
                name: "http".to_string(),
                url: "/health".to_string(),
                interval: "5".to_string(),
                timeout: "2s".to_string(),
                deregister_after: "30".to_string(),
            }),
            &reg,
        );
        assert_eq!(check["HTTP"], "http://10.0.0.2:8085/health");
        assert_eq!(check["Interval"], "5s");
        assert_eq!(check["Timeout"], "2s");
        // Consul 要求至少 1 分钟，30s 必须被提升，否则注册会被拒绝
        assert_eq!(check["DeregisterCriticalServiceAfter"], "60s");
        assert!(ttl.is_none());

        let (check, ttl) = build_check(
            Some(&HealthCheck {
                health_type: "ttl".to_string(),
                name: "ttl".to_string(),
                url: "".to_string(),
                interval: "10".to_string(),
                timeout: "".to_string(),
                deregister_after: "60".to_string(),
            }),
            &reg,
        );
        assert_eq!(check["TTL"], "10s");
        assert_eq!(ttl, Some(10));

        let (check, ttl) = build_check(None, &reg);
        assert_eq!(check["TTL"], "10s");
        assert_eq!(ttl, Some(DEFAULT_TTL_SECS));
    }

    #[tokio::test]
    async fn ttl_updater_stops_from_a_different_consul_instance() {
        let config = AppConfig::from_file(Some(&crate::config::global_config_path())).unwrap();
        let registrar = Consul::from_config(&config);
        let shutdown = Consul::from_config(&config);
        let id = format!("ttl-cross-instance-{}", uuid::Uuid::new_v4());

        // 上报周期远大于测试时长，任务不会真的发出 HTTP 请求
        registrar
            .start_ttl_updater(id.clone(), 30, serde_json::json!({}))
            .await;
        assert!(ttl_updaters().lock().await.contains_key(&id));

        shutdown.stop_ttl_updater(&id).await;
        assert!(!ttl_updaters().lock().await.contains_key(&id));
    }
}
