use config::{Config, ConfigError, File, FileFormat};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;
use crate::configs::{DatabaseConfig, GatewayConfig, LogConfig, MessageStorageConfig, OssConfig, TelemetryConfig, SmsConfig};
use crate::utils::url;

// 配置只经 AppConfig::from_file → 构造注入传递，不再保留进程级可变静态。

/// 解析全局配置文件路径
///
/// 优先使用运行目录下的 `config/config.yaml`（服务从 workspace 根启动的场景）；
/// 不存在时（如 `cargo test` 以 crate 目录为工作目录）回退到 manifest 相对路径。
pub fn global_config_path() -> String {
    if Path::new("./config/config.yaml").exists() {
        "./config/config.yaml".to_string()
    } else {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../config/config.yaml").to_string()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub gateway: GatewayConfig, //网关配置
    pub component: Component,
    pub log: LogConfig,
    pub telemetry: TelemetryConfig,  // 链路追踪配置
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub service_center: ServiceCenterConfig,
    pub websocket: WebsocketConfig,
    pub rpc: RpcConfig,
    pub redis: RedisConfig,
    pub kafka: KafkaConfig,
    pub oss: OssConfig,
    pub sms: SmsConfig,
    /// 消息存储分流配置
    ///
    /// 允许整个段落缺省（现有配置文件无需改动即可启动），
    /// 缺省时使用 [`MessageStorageConfig::default`] 的阈值。
    #[serde(default)]
    pub message_storage: MessageStorageConfig,
    /// 消息发送权限校验配置
    ///
    /// 允许整个段落缺省，缺省时采用严格模式（校验失败即拒绝）。
    #[serde(default)]
    pub validation: ValidationConfig,
    /// 注册到 Consul 时对外广告的地址，由 `ADVERTISE_HOST` 注入
    ///
    /// 配置文件里的 host 是本地监听地址（127.0.0.1）；在容器中若原样注册，
    /// 服务发现会把流量导向调用方自己。Pod 环境必须注入 Pod IP。
    #[serde(default)]
    pub advertise_host: Option<String>,
    /// Prometheus 指标导出配置（整段可缺省，缺省即关闭）
    #[serde(default)]
    pub metrics: MetricsConfig,
}

/// Prometheus 指标导出配置
///
/// 只有 msg-server / msg-gateway 会用到：它们没有 HTTP 业务端点，
/// 需要单独起一个 `/metrics` 监听。api-gateway 用自己的路由与 recorder，忽略本段。
#[derive(Debug, Deserialize, Clone, Default)]
pub struct MetricsConfig {
    /// 是否启动指标导出。缺省 false：旧配置文件升级后不会平白多占端口
    #[serde(default)]
    pub enabled: bool,
    /// 监听端口；0 表示用服务内置默认端口（msg-server 9101 / msg-gateway 9102）
    #[serde(default)]
    pub port: u16,
}

/// 消息发送权限校验配置
#[derive(Debug, Deserialize, Clone)]
pub struct ValidationConfig {
    /// 校验服务故障时是否放行（fail-open）
    ///
    /// - false（默认，fail-closed）：缓存与RPC都不可用时拒绝发送，安全性优先；
    /// - true（fail-open）：校验链路故障时放行消息，可用性优先。
    ///   仅建议在消息安全等级要求不高的部署中开启。
    #[serde(default)]
    pub fail_open: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self { fail_open: false }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    pub host: String,
    pub port: u16,
    pub seq_step: i32,
    pub max_connections: Option<usize>,
    pub pool_timeout_ms: Option<u64>,
    pub connection_timeout_ms: Option<u64>,
}

impl RedisConfig {
    pub fn url(&self) -> String {
        format!("redis://{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaProducerConfig {
    pub timeout: u64,
    pub acks: String,
    pub max_retry: u32,
    pub retry_interval: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaConsumerConfig {
    pub auto_offset_reset: String,
    pub session_timeout: u64,
    /// 单实例内的消费者并行度
    ///
    /// 同一消费组内运行 N 个消费者，分区在它们之间自动均衡分配；
    /// 每个消费者内部再有「批内并行、组内保序」处理。缺省 4。
    #[serde(default = "default_consumer_parallelism")]
    pub parallelism: usize,
}

fn default_consumer_parallelism() -> usize {
    4
}

impl KafkaConsumerConfig {
    /// 实际生效的消费者并行度（至少 1）
    pub fn effective_parallelism(&self) -> usize {
        self.parallelism.max(1)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaConfig {
    pub hosts: Vec<String>,
    pub topic: String,
    pub group: String,
    pub connect_timeout: u64,
    pub producer: KafkaProducerConfig,
    pub consumer: KafkaConsumerConfig,
    /// 消息主题的分区数
    ///
    /// 消费端按会话哈希并行处理的前提是分区可并行；分区数同时决定
    /// 单个消费实例内部的最大并行度。缺省 16，0/负数视为非法回落默认值。
    #[serde(default = "default_topic_partitions")]
    pub topic_partitions: i32,
    /// 消息主题的副本数
    ///
    /// 生产环境应 ≥3（配合 `min.insync.replicas=2`），
    /// 单副本在 broker 故障时主题不可写。缺省 3，最少 1。
    #[serde(default = "default_topic_replication")]
    pub topic_replication: i32,
}

fn default_topic_partitions() -> i32 {
    16
}

fn default_topic_replication() -> i32 {
    3
}

impl KafkaConfig {
    /// 实际生效的分区数（至少 1）
    pub fn effective_partitions(&self) -> i32 {
        if self.topic_partitions < 1 {
            default_topic_partitions()
        } else {
            self.topic_partitions
        }
    }

    /// 实际生效的副本数（至少 1）
    pub fn effective_replication(&self) -> i32 {
        if self.topic_replication < 1 {
            default_topic_replication()
        } else {
            self.topic_replication
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct JwtConfig {
    pub secret: String,
    pub expiration: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub ws_lb_strategy: String,
    /// TLS/HTTPS配置
    pub tls: Option<TlsConfig>,
    /// 超时配置
    pub timeouts: Option<TimeoutConfig>,
    /// 连接配置
    pub connection: Option<ConnectionConfig>,
}

/// TLS配置
#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    /// 是否启用HTTPS
    pub enabled: bool,
    /// 证书文件路径
    pub cert_file: String,
    /// 私钥文件路径
    pub key_file: String,
    /// CA证书文件路径（可选）
    pub client_ca_file: Option<String>,
    /// 是否要求客户端证书
    pub require_client_cert: bool,
}

/// 超时配置
#[derive(Debug, Deserialize, Clone)]
pub struct TimeoutConfig {
    /// 读取超时（秒）
    pub read_timeout_secs: u64,
    /// 写入超时（秒）
    pub write_timeout_secs: u64,
    /// 空闲连接超时（秒）
    pub idle_timeout_secs: u64,
}

/// 连接配置
#[derive(Debug, Deserialize, Clone)]
pub struct ConnectionConfig {
    /// 最大连接数
    pub max_connections: u32,
    /// Keep-Alive超时（秒）
    pub keep_alive_timeout_secs: u64,
    /// 禁用Nagle算法
    pub tcp_nodelay: bool,
}

impl ServerConfig {
    pub fn url(&self, https: bool) -> String {
        url(https, &self.host, self.port)
    }
    pub fn server_url(&self) -> String {
        format!("{}:{}", &self.host, self.port)
    }

    pub fn with_port(&self, port: u16) -> ServerConfig {
        ServerConfig {
            host: self.host.clone(),
            port,
            ws_lb_strategy: self.ws_lb_strategy.clone(),
            tls: self.tls.clone(),
            timeouts: self.timeouts.clone(),
            connection: self.connection.clone(),
        }
    }
}

/// 服务发现配置
#[derive(Debug, Deserialize, Clone)]
pub struct ServiceCenterConfig {
    pub host: String,
    pub port: u16,
    /// 注册中心 HTTP 请求超时（秒）
    ///
    /// 历史上这里被误填为 8080（端口号），依赖不可达时请求会挂起两小时以上，
    /// 直接击穿 RTO，因此代码侧再限幅一次。
    pub timeout: u64,
    pub protocol: String,
    /// 发现端刷新间隔（秒），决定故障实例被摘流的最大延迟
    #[serde(default = "default_refresh_interval")]
    pub service_refresh_interval: u64,
}

fn default_refresh_interval() -> u64 {
    5
}

impl ServiceCenterConfig {
    /// 请求超时限幅 [1, 10] 秒，防止配置误填放大到小时级
    pub fn request_timeout(&self) -> u64 {
        self.timeout.clamp(1, 10)
    }

    /// 发现刷新间隔限幅 [1, 30] 秒
    pub fn refresh_interval(&self) -> u64 {
        self.service_refresh_interval.clamp(1, 30)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebsocketConfig {
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub name: String,
    pub tags: Vec<String>,
    pub health_check: Option<HealthCheckConfig>,
    /// 上行消息处理 worker 数
    ///
    /// 所有连接的上行消息按会话哈希分散到 N 个 worker 并行处理，
    /// 同一会话的消息始终落在同一 worker 上保证处理顺序。
    /// 缺省 8；0 视为非法，回落默认值。
    #[serde(default)]
    pub upstream_workers: Option<usize>,
    /// 心跳死链判定阈值：连续 N 个心跳周期未收到 Pong 即断开连接
    ///
    /// 缺省 2（即约 2×心跳间隔 ≈ 60s 无响应判定死链）。
    #[serde(default)]
    pub heartbeat_max_missed: Option<u32>,
}

impl WebsocketConfig {
    /// 实际生效的上行 worker 数
    pub fn worker_count(&self) -> usize {
        const DEFAULT_WORKERS: usize = 8;
        match self.upstream_workers {
            Some(n) if n > 0 => n,
            _ => DEFAULT_WORKERS,
        }
    }

    /// 实际生效的心跳死链阈值（至少 1）
    pub fn heartbeat_max_missed(&self) -> u32 {
        match self.heartbeat_max_missed {
            Some(n) if n >= 1 => n,
            _ => 2,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct HealthCheckConfig {
    pub health_type: String,
    pub name: String,
    pub url: String,
    pub interval: u64,
    pub timeout: u64,
    pub deregister_after: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GrpcHealthCheckConfig {
    pub grpc_use_tls: bool,
    pub interval: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RpcServiceConfig {
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub name: String,
    pub tags: Vec<String>,
    pub grpc_health_check: Option<GrpcHealthCheckConfig>,
    pub health_check: Option<HealthCheckConfig>,
}

impl RpcServiceConfig {
    #[inline]
    pub fn rpc_server_url(&self) -> String {
        url(false, &self.host, self.port);
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RpcConfig {
    pub ws: RpcServiceConfig,
    pub chat: RpcServiceConfig,
    pub user: RpcServiceConfig,
    pub friend: RpcServiceConfig,
    pub group: RpcServiceConfig,
}

/// RPC服务注册枚举
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Component {
    UserServer,
    FriendServer,
    GroupServer,
    /// 用户 / 好友 / 群组的模块化单体
    ///
    /// 三者同进程、同端口、只注册一次。保留 User/Friend/Group 变体是为了
    /// 兼容旧的配置文件路径与部署脚本，新部署应统一使用本变体。
    RelationServer,
    MessageServer,
    MessageGateway,
    All,
}

impl AppConfig {
    // 创建一个新的AppConfig实例
    pub fn new() -> Result<Self, ConfigError> {
        Self::from_file(None)
    }

    // 从多个来源加载配置
    pub fn from_file(file_path: Option<&str>) -> Result<Self, ConfigError> {
        // 开始构建配置
        let mut builder = Config::builder();

        // 2. 配置文件 (如果指定)
        if let Some(path) = file_path {
            if Path::new(path).exists() {
                let format = if path.ends_with(".json") {
                    FileFormat::Json
                } else if path.ends_with(".yaml") || path.ends_with(".yml") {
                    FileFormat::Yaml
                } else {
                    FileFormat::Toml
                };

                builder = builder.add_source(File::with_name(path).format(format));
            }
        } else {
            // 1. 默认配置文件
            builder = builder.add_source(File::with_name("./config/config.yaml").format(FileFormat::Yaml));
        }

        // 4. 读取环境变量 (最高优先级)
        builder = builder.add_source(config::Environment::default().separator(","));

        // 构建配置
        let config = builder.build()?;

        // 转换为AppConfig结构体
        let mut app_config: AppConfig = config.try_deserialize()?;
        app_config.apply_env_overrides();
        Ok(app_config)
    }

    /// 服务注册时对外广告的地址：容器环境取 `ADVERTISE_HOST`，否则沿用监听地址
    pub fn advertised_host(&self, bind_host: &str) -> String {
        self.advertise_host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or(bind_host)
            .to_string()
    }

    /// 环境变量覆盖
    ///
    /// 敏感密钥不落盘：`OSS_ACCESS_KEY` / `OSS_SECRET_KEY` / `SMS_SECRET_ID` /
    /// `SMS_SECRET_KEY` / `JWT_SECRET`；容器编排相关：`ADVERTISE_HOST`（Pod IP，
    /// 用于服务注册）、`CONSUL_HOST` / `CONSUL_PORT`（注册中心地址）。
    fn apply_env_overrides(&mut self) {
        let overrides: [(&str, &mut String); 5] = [
            ("OSS_ACCESS_KEY", &mut self.oss.access_key),
            ("OSS_SECRET_KEY", &mut self.oss.secret_key),
            ("SMS_SECRET_ID", &mut self.sms.tencent.secret_id),
            ("SMS_SECRET_KEY", &mut self.sms.tencent.secret_key),
            ("JWT_SECRET", &mut self.gateway.auth.jwt.secret),
        ];
        for (env_key, target) in overrides {
            if let Ok(value) = std::env::var(env_key) {
                if !value.trim().is_empty() {
                    info!("已从环境变量 {} 覆盖敏感配置", env_key);
                    *target = value;
                }
            }
        }

        if let Ok(value) = std::env::var("ADVERTISE_HOST") {
            if !value.trim().is_empty() {
                self.advertise_host = Some(value);
            }
        }
        if let Ok(value) = std::env::var("CONSUL_HOST") {
            if !value.trim().is_empty() {
                self.service_center.host = value;
            }
        }
        if let Ok(value) = std::env::var("CONSUL_PORT") {
            if let Ok(port) = value.parse::<u16>() {
                self.service_center.port = port;
            }
        }
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load() {
        let config = match AppConfig::from_file(Some(&global_config_path())) {
            Ok(config) => config,
            Err(err) => {
                panic!("load config error: {:?}", err);
            }
        };
        println!("{:?}", config);
        assert_eq!(config.database.postgres.host, "127.0.0.1");
        assert_eq!(config.database.postgres.port, 5432);
        assert_eq!(config.database.postgres.user, "kelisi");
        assert_eq!(config.database.postgres.password, "123456");
    }

    #[test]
    fn service_center_timeouts_are_bounded() {
        let config = AppConfig::from_file(Some(&global_config_path())).unwrap();
        assert!(config.service_center.request_timeout() <= 10);
        assert!((1..=30).contains(&config.service_center.refresh_interval()));
    }

    #[test]
    fn advertised_host_prefers_injected_pod_ip() {
        let mut config = AppConfig::from_file(Some(&global_config_path())).unwrap();
        config.advertise_host = None;
        assert_eq!(config.advertised_host("0.0.0.0"), "0.0.0.0");
        config.advertise_host = Some("10.1.2.3".to_string());
        assert_eq!(config.advertised_host("0.0.0.0"), "10.1.2.3");
        config.advertise_host = Some("  ".to_string());
        assert_eq!(config.advertised_host("0.0.0.0"), "0.0.0.0");
    }

}
