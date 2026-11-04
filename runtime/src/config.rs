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
    pub timeout: u64,
    pub protocol: String,
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
        app_config.apply_secret_env_overrides();
        Ok(app_config)
    }

    /// 敏感信息环境变量覆盖
    ///
    /// 历史上云存储/短信密钥与JWT密钥直接写进 config.yaml，
    /// 存在随仓库泄露的风险。以下环境变量（存在且非空时）优先于配置文件：
    /// `OSS_ACCESS_KEY` / `OSS_SECRET_KEY` / `SMS_SECRET_ID` /
    /// `SMS_SECRET_KEY` / `JWT_SECRET`
    fn apply_secret_env_overrides(&mut self) {
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

}
