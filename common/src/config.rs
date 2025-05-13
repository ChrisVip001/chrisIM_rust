use config::{Config, ConfigError, File, FileFormat};
use dotenv::dotenv;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize, Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MongodbConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: String,
    pub clean: MongodbCleanConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MongodbCleanConfig {
    pub period: u64,
    pub except_types: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub postgres: PostgresConfig,
    pub mongodb: MongodbConfig,
    pub xdb: String,
}

impl DatabaseConfig {
    pub fn url(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.postgres.user,
            self.postgres.password,
            self.postgres.host,
            self.postgres.port,
            self.postgres.database
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    pub host: String,
    pub port: u16,
    pub seq_step: i32,
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaConfig {
    pub hosts: Vec<String>,
    pub topic: String,
    pub group: String,
    pub connect_timeout: u64,
    pub producer: KafkaProducerConfig,
    pub consumer: KafkaConsumerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JwtConfig {
    pub secret: String,
    pub expiration: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Oauth2Provider {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_url: String,
    pub user_info_url: String,
    pub email_url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Oauth2Config {
    pub google: Oauth2Provider,
    pub github: Oauth2Provider,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub ws_lb_strategy: String,
    pub oauth2: Oauth2Config,
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
            oauth2: self.oauth2.clone(),
        }
    }
}

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
}

impl WebsocketConfig {
    #[inline]
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.protocol, self.host, self.port)
    }

    #[inline]
    pub fn url_with_protocol(&self, https: bool) -> String {
        url(https, &self.host, self.port)
    }

    #[inline]
    pub fn ws_url(&self, secure: bool) -> String {
        if secure {
            format!("wss://{}:{}", self.host, self.port)
        } else {
            format!("ws://{}:{}", self.host, self.port)
        }
    }
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
}

impl RpcServiceConfig {
    #[inline]
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.protocol, self.host, self.port)
    }

    #[inline]
    pub fn rpc_server_url(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    #[inline]
    pub fn url_with_protocol(&self, https: bool) -> String {
        url(https, &self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RpcConfig {
    pub health_check: bool,
    pub ws: RpcServiceConfig,
    pub chat: RpcServiceConfig,
    pub db: RpcServiceConfig,
    pub pusher: RpcServiceConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MailConfig {
    pub server: String,
    pub account: String,
    pub password: String,
    pub temp_path: String,
    pub temp_file: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    pub level: String,
    pub output: String,
}

impl LogConfig {
    pub fn level(&self) -> tracing::Level {
        match self.level.as_str() {
            "trace" => tracing::Level::TRACE,
            "debug" => tracing::Level::DEBUG,
            "info" => tracing::Level::INFO,
            "warn" => tracing::Level::WARN,
            "error" => tracing::Level::ERROR,
            _ => tracing::Level::INFO,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub component: Component,
    pub log: LogConfig,
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub service_center: ServiceCenterConfig,
    pub websocket: WebsocketConfig,
    pub rpc: RpcConfig,
    pub redis: RedisConfig,
    pub kafka: KafkaConfig,
    pub jwt: JwtConfig,
    pub oss: OssConfig,
    pub mail: MailConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OssConfig {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub avatar_bucket: String,
    pub region: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Component {
    Api,
    Ws,
    Rpc,
    Db,
    Pusher,
    All,
}

// 封装配置以支持动态更新
pub struct DynamicConfig {
    current: RwLock<Arc<AppConfig>>,
    config_paths: Vec<String>,
    refresh_interval: Duration,
}

impl AppConfig {
    // 创建一个新的AppConfig实例
    pub fn new() -> Result<Self, ConfigError> {
        Self::from_file(None)
    }

    // 从多个来源加载配置
    pub fn from_file(file_path: Option<&str>) -> Result<Self, ConfigError> {
        // 尝试加载.env文件，但不要求它必须存在
        dotenv().ok();

        // 开始构建配置
        let mut builder = Config::builder();

        // 1. 默认配置
        builder = builder
            .set_default("component", "all")?
            .set_default("log.level", "debug")?
            .set_default("log.output", "console")?
            .set_default("database.postgres.host", "127.0.0.1")?
            .set_default("database.postgres.port", 5432)?
            .set_default("database.postgres.user", "kelisi")?
            .set_default("database.postgres.password", "123456")?
            .set_default("database.postgres.database", "rustim")?
            .set_default("database.mongodb.host", "127.0.0.1")?
            .set_default("database.mongodb.port", 27017)?
            .set_default("database.mongodb.database", "im")?
            .set_default("database.mongodb.clean.period", 3600)?
            .set_default("database.mongodb.clean.except_types", Vec::<String>::new())?
            .set_default("database.xdb", "./api/fixtures/xdb/ip2region.xdb")?
            .set_default("server.host", "127.0.0.1")?
            .set_default("server.port", 50001)?
            .set_default("server.ws_lb_strategy", "RoundRobin")?
            .set_default("service_center.host", "127.0.0.1")?
            .set_default("service_center.port", 8500)?
            .set_default("service_center.timeout", 5000)?
            .set_default("service_center.protocol", "http")?
            .set_default("websocket.protocol", "ws")?
            .set_default("websocket.host", "127.0.0.1")?
            .set_default("websocket.port", 50000)?
            .set_default("websocket.name", "websocket")?
            .set_default(
                "websocket.tags",
                vec!["websocket".to_string(), "grpc".to_string()],
            )?
            .set_default("rpc.health_check", false)?
            .set_default("rpc.ws.protocol", "http")?
            .set_default("rpc.ws.host", "127.0.0.1")?
            .set_default("rpc.ws.port", 50002)?
            .set_default("rpc.ws.name", "ws")?
            .set_default("rpc.ws.tags", vec!["ws".to_string(), "grpc".to_string()])?
            .set_default("rpc.chat.protocol", "http")?
            .set_default("rpc.chat.host", "127.0.0.1")?
            .set_default("rpc.chat.port", 50003)?
            .set_default("rpc.chat.name", "chat")?
            .set_default(
                "rpc.chat.tags",
                vec!["chat".to_string(), "grpc".to_string()],
            )?
            .set_default("rpc.db.protocol", "http")?
            .set_default("rpc.db.host", "127.0.0.1")?
            .set_default("rpc.db.port", 50004)?
            .set_default("rpc.db.name", "db")?
            .set_default("rpc.db.tags", vec!["db".to_string(), "grpc".to_string()])?
            .set_default("rpc.pusher.protocol", "http")?
            .set_default("rpc.pusher.host", "127.0.0.1")?
            .set_default("rpc.pusher.port", 50005)?
            .set_default("rpc.pusher.name", "pusher")?
            .set_default(
                "rpc.pusher.tags",
                vec!["pusher".to_string(), "grpc".to_string()],
            )?
            .set_default("redis.host", "127.0.0.1")?
            .set_default("redis.port", 6379)?
            .set_default("redis.seq_step", 10000)?
            .set_default("kafka.hosts", vec!["127.0.0.1:9092".to_string()])?
            .set_default("kafka.topic", "rustIM-chat")?
            .set_default("kafka.group", "chat")?
            .set_default("kafka.connect_timeout", 5000)?
            .set_default("kafka.producer.timeout", 3000)?
            .set_default("kafka.producer.acks", "all")?
            .set_default("kafka.producer.max_retry", 3)?
            .set_default("kafka.producer.retry_interval", 1000)?
            .set_default("kafka.consumer.auto_offset_reset", "earliest")?
            .set_default("kafka.consumer.session_timeout", 20000)?
            .set_default(
                "jwt.secret",
                "development_jwt_secret_do_not_use_in_production",
            )?
            .set_default("jwt.expiration", 86400)?
            .set_default("oss.endpoint", "http://127.0.0.1:9000")?
            .set_default("oss.access_key", "minioadmin")?
            .set_default("oss.secret_key", "minioadmin")?
            .set_default("oss.bucket", "rustIM")?
            .set_default("oss.avatar_bucket", "rustIM-avatar")?
            .set_default("oss.region", "us-east-1")?
            .set_default("mail.server", "smtp.qq.com")?
            .set_default("mail.account", "17788889999@qq.com")?
            .set_default("mail.password", "iejtiohyreybgdf")?
            .set_default("mail.temp_path", "./api/fixtures/templates/*")?
            .set_default("mail.temp_file", "email_temp.html")?;

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
        }

        // 3. 检查默认的配置文件路径
        for path in [
            "config.toml",
            "config.yaml",
            "config.yml",
            "config.json",
            "./config/config.yaml",
            ".env",
        ] {
            if Path::new(path).exists() {
                let format = if path.ends_with(".json") {
                    FileFormat::Json
                } else if path.ends_with(".yaml") || path.ends_with(".yml") {
                    FileFormat::Yaml
                } else if path.ends_with(".toml") {
                    FileFormat::Toml
                } else {
                    // .env 文件默认使用环境变量格式
                    continue;
                };

                builder = builder.add_source(File::with_name(path).format(format));
            }
        }

        // 4. 读取环境变量 (最高优先级)
        builder = builder.add_source(config::Environment::default().separator("_"));

        // 构建配置
        let config = builder.build()?;

        // 转换为AppConfig结构体
        Ok(config.try_deserialize()?)
    }
}

impl DynamicConfig {
    // 创建一个新的动态配置实例
    pub fn new(
        config_paths: Vec<String>,
        refresh_interval_secs: u64,
    ) -> Result<Self, ConfigError> {
        let interval = Duration::from_secs(refresh_interval_secs);
        let config = AppConfig::new()?;

        Ok(DynamicConfig {
            current: RwLock::new(Arc::new(config)),
            config_paths,
            refresh_interval: interval,
        })
    }

    // 获取当前配置
    pub fn get_config(&self) -> Arc<AppConfig> {
        self.current.read().unwrap().clone()
    }

    // 启动配置监控线程
    pub fn start_refresh_task(self: Arc<Self>) {
        let dynamic_config = self.clone();

        thread::spawn(move || {
            info!(
                "配置监控线程启动，刷新间隔: {:?}",
                dynamic_config.refresh_interval
            );

            loop {
                thread::sleep(dynamic_config.refresh_interval);
                match dynamic_config.refresh_config() {
                    Ok(_) => info!("配置已更新"),
                    Err(e) => error!("刷新配置失败: {}", e),
                }
            }
        });
    }

    // 刷新配置
    fn refresh_config(&self) -> Result<(), ConfigError> {
        for path in &self.config_paths {
            if !Path::new(path).exists() {
                continue;
            }

            // 尝试从配置文件加载新配置
            match AppConfig::from_file(Some(path)) {
                Ok(new_config) => {
                    // 更新当前配置
                    let mut current = self.current.write().unwrap();
                    *current = Arc::new(new_config);
                    info!("已从文件 {} 加载新配置", path);
                    return Ok(());
                }
                Err(e) => {
                    warn!("从 {} 加载配置失败: {}", path, e);
                }
            }
        }

        // 如果所有路径都失败，尝试从环境变量加载
        match AppConfig::new() {
            Ok(new_config) => {
                let mut current = self.current.write().unwrap();
                *current = Arc::new(new_config);
                info!("已从环境变量加载新配置");
                Ok(())
            }
            Err(e) => {
                error!("刷新配置失败: {}", e);
                Err(e)
            }
        }
    }
}

// 辅助函数，用于构建URL字符串
fn url(https: bool, host: &str, port: u16) -> String {
    if https {
        format!("https://{}:{}", host, port)
    } else {
        format!("http://{}:{}", host, port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load() {
        let config = match AppConfig::from_file(Some("./config/config.yaml")) {
            Ok(config) => config,
            Err(err) => {
                panic!("load config error: {:?}", err);
            }
        };
        println!("{:?}", config);
        assert_eq!(config.database.postgres.host, "localhost");
        assert_eq!(config.database.postgres.port, 5432);
        assert_eq!(config.database.postgres.user, "kelisi");
        assert_eq!(config.database.postgres.password, "123456");
    }
}
