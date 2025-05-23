use config::{Config, ConfigError, File, FileFormat};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock};
#[cfg(feature = "dynamic-config")]
use notify::{Event, RecursiveMode, Watcher};
use once_cell::sync::Lazy;
use tracing::{error, info, warn};
use crate::configs::{DatabaseConfig, GatewayConfig, LogConfig, OssConfig, TelemetryConfig};
use crate::utils::url;

// 定义一个静态全局配置，可以在任何地方访问
pub static GLOBAL_CONFIG: Lazy<RwLock<Option<Arc<AppConfig>>>> = Lazy::new(|| RwLock::new(None));

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
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub ws_lb_strategy: String,
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
    pub fn rpc_server_url(&self) -> String {
        url(false, &self.host, self.port);
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RpcConfig {
    pub api: RpcServiceConfig,
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
    ApiGateway,
    UserServer,
    FriendServer,
    GroupServer,
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
        builder = builder.add_source(config::Environment::default().separator("_"));

        // 构建配置
        let config = builder.build()?;

        // 转换为AppConfig结构体
        Ok(config.try_deserialize()?)
    }

    // 新增: 根据服务类型获取服务特定的配置文件路径
    fn get_service_config_path(component: &Component) -> Option<String> {
        match component {
            Component::ApiGateway => Some("./config/services/api-gateway.yaml".to_string()),
            Component::UserServer => Some("./config/services/user-service.yaml".to_string()),
            Component::FriendServer => Some("./config/services/friend-service.yaml".to_string()),
            Component::GroupServer => Some("./config/services/group-service.yaml".to_string()),
            Component::MessageServer => Some("./config/services/msg-server.yaml".to_string()),
            Component::MessageGateway => Some("./config/services/msg-gateway.yaml".to_string()),
            Component::All => Some("./config/config.yaml".to_string()), // 全局配置
        }
    }
}

// 新增: 提供更强大的配置加载功能，结合全局和服务特定配置
pub struct ConfigLoader {
    global_config: Option<Arc<AppConfig>>,
    service_config: Option<Arc<AppConfig>>,
    component: Component,
}

impl ConfigLoader {
    // 创建一个新的配置加载器实例
    pub fn new(component: Component) -> Self {
        Self {
            global_config: None,
            service_config: None,
            component,
        }
    }

    // 加载配置，先加载全局配置，然后加载服务特定配置并合并
    pub fn load(&mut self) -> Result<Arc<AppConfig>, ConfigError> {
        // 1. 加载全局配置
        let global_config = AppConfig::from_file(Some("./config/config.yaml"))?;
        self.global_config = Some(Arc::new(global_config));

        // 2. 尝试加载服务特定配置
        if let Some(service_path) = AppConfig::get_service_config_path(&self.component) {
            if Path::new(&service_path).exists() {
                match AppConfig::from_file(Some(&service_path)) {
                    Ok(service_config) => {
                        self.service_config = Some(Arc::new(service_config));
                        info!("已从 {} 加载服务特定配置", service_path);
                    }
                    Err(e) => {
                        warn!("加载服务特定配置失败 {}: {}", service_path, e);
                    }
                }
            }
        }

        // 3. 返回合并后的配置
        Ok(self.get_merged_config())
    }

    // 获取合并后的配置
    pub fn get_merged_config(&self) -> Arc<AppConfig> {
        // 如果没有服务特定配置，直接返回全局配置
        if self.service_config.is_none() {
            return self.global_config.clone().unwrap_or_else(|| Arc::new(AppConfig::new().unwrap()));
        }

        // 如果有服务特定配置，需要智能合并
        let global = self.global_config.clone().unwrap();
        let service = self.service_config.clone().unwrap();

        // 创建新的合并配置，以全局配置为基础
        let mut merged_config = (*global).clone();

        // 合并服务器配置
        Self::merge_server_config(&mut merged_config.server, &service.server);
        
        // 合并数据库配置
        Self::merge_database_config(&mut merged_config.database, &service.database);
        
        // 合并日志配置
        Self::merge_log_config(&mut merged_config.log, &service.log);
        
        // 合并Redis配置
        Self::merge_redis_config(&mut merged_config.redis, &service.redis);
        
        // 确保组件类型正确设置
        merged_config.component = service.component.clone();

        Arc::new(merged_config)
    }

    // 合并数据库配置
    fn merge_database_config(target: &mut DatabaseConfig, source: &DatabaseConfig) {
        // 合并Postgres配置
        if !source.postgres.host.is_empty() {
            target.postgres.host = source.postgres.host.clone();
        }
        if source.postgres.port != 0 {
            target.postgres.port = source.postgres.port;
        }
        if !source.postgres.user.is_empty() {
            target.postgres.user = source.postgres.user.clone();
        }
        if !source.postgres.password.is_empty() {
            target.postgres.password = source.postgres.password.clone();
        }
        if !source.postgres.database.is_empty() {
            target.postgres.database = source.postgres.database.clone();
        }

        // 合并MongoDB配置
        if !source.mongodb.host.is_empty() {
            target.mongodb.host = source.mongodb.host.clone();
        }
        if source.mongodb.port != 0 {
            target.mongodb.port = source.mongodb.port;
        }
        if Some(source.mongodb.user.as_ref()) != None {
            target.mongodb.user = source.mongodb.user.clone();
        }
        if Some (source.mongodb.password.as_ref()) != None{ 
            target.mongodb.password = source.mongodb.password.clone();
        }
        if !source.mongodb.database.is_empty() {
            target.mongodb.database = source.mongodb.database.clone();
        }
        
        // 合并clean配置
        if source.mongodb.clean.period != 0 {
            target.mongodb.clean.period = source.mongodb.clean.period;
        }
        if !source.mongodb.clean.except_types.is_empty() {
            target.mongodb.clean.except_types = source.mongodb.clean.except_types.clone();
        }
    }
    
    // 合并服务器配置
    fn merge_server_config(target: &mut ServerConfig, source: &ServerConfig) {
        if !source.host.is_empty() {
            target.host = source.host.clone();
        }
        if source.port != 0 {
            target.port = source.port;
        }
        if !source.ws_lb_strategy.is_empty() {
            target.ws_lb_strategy = source.ws_lb_strategy.clone();
        }
    }
    
    // 合并日志配置
    fn merge_log_config(target: &mut LogConfig, source: &LogConfig) {
        if !source.level.is_empty() {
            target.level = source.level.clone();
        }
        if !source.output.is_empty() {
            target.output = source.output.clone();
        }
        if let Some(format) = &source.format { 
            target.format = Some(format.clone());
        }
        if let Some(sqlx_level) = &source.sqlx_level { 
            target.sqlx_level = Some(sqlx_level.clone());
        }
        
        // 合并组件级别日志配置
        if let Some(source_components) = &source.components {
            // 如果目标 components 为 None，初始化它
            if target.components.is_none() {
                target.components = Some(std::collections::HashMap::new());
            }
            
            // 现在可以安全地获取 target.components 的可变引用并插入元素
            if let Some(target_components) = &mut target.components {
                for (component, level) in source_components {
                    target_components.insert(component.clone(), level.clone());
                }
            }
        }
    }
    
    // 合并Redis配置
    fn merge_redis_config(target: &mut RedisConfig, source: &RedisConfig) {
        if !source.host.is_empty() {
            target.host = source.host.clone();
        }
        if source.port != 0 {
            target.port = source.port;
        }
        if source.seq_step != 0 {
            target.seq_step = source.seq_step;
        }
        if let Some(max_conn) = source.max_connections {
            target.max_connections = Some(max_conn);
        }
        if let Some(pool_timeout) = source.pool_timeout_ms {
            target.pool_timeout_ms = Some(pool_timeout);
        }
        if let Some(conn_timeout) = source.connection_timeout_ms {
            target.connection_timeout_ms = Some(conn_timeout);
        }
    }
    
    // 合并JWT配置
    fn merge_jwt_config(target: &mut JwtConfig, source: &JwtConfig) {
        if !source.secret.is_empty() {
            target.secret = source.secret.clone();
        }
        if source.expiration != 0 {
            target.expiration = source.expiration;
        }
    }

    // 初始化全局配置单例
    pub fn init_global() -> Result<(), ConfigError> {
        let global_config = AppConfig::from_file(Some("./config/config.yaml"))?;
        let mut config_guard = GLOBAL_CONFIG.write().unwrap();
        *config_guard = Some(Arc::new(global_config));
        Ok(())
    }

    // 获取全局配置单例
    pub fn get_global() -> Option<Arc<AppConfig>> {
        GLOBAL_CONFIG.read().unwrap().clone()
    }

    // 设置新的全局配置
    pub fn set_global(config: AppConfig) {
        let mut config_guard = GLOBAL_CONFIG.write().unwrap();
        *config_guard = Some(Arc::new(config));
    }

    #[cfg(feature = "dynamic-config")]
    // 监控配置文件变化并自动重新加载
    pub fn watch_config_changes(component: Component) -> Result<(), anyhow::Error> {
        // 为闭包创建一个克隆，这样原始的component不会被移动
        let component_for_closure = component.clone();
        
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    info!("配置文件变更: {:?}", event);
                    // 重新加载配置
                    match ConfigLoader::reload_config(component_for_closure.clone()) {
                        Ok(_) => info!("成功重新加载配置"),
                        Err(e) => error!("重新加载配置失败: {}", e),
                    }
                }
                Err(e) => error!("监控配置文件错误: {}", e),
            }
        })?;

        // 监控全局配置文件
        watcher.watch(Path::new("./config/config.yaml"), RecursiveMode::NonRecursive)?;

        // 监控服务特定配置文件
        if let Some(service_path) = AppConfig::get_service_config_path(&component) {
            if Path::new(&service_path).exists() {
                watcher.watch(Path::new(&service_path), RecursiveMode::NonRecursive)?;
            }
        }

        Ok(())
    }

    // 重新加载配置
    fn reload_config(component: Component) -> Result<(), ConfigError> {
        let mut loader = ConfigLoader::new(component);
        let config = loader.load()?;
        ConfigLoader::set_global((*config).clone());
        Ok(())
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

    // 测试配置加载器和全局配置单例
    #[test]
    fn test_config_loader() {
        // 初始化全局配置
        ConfigLoader::init_global().expect("初始化全局配置失败");

        // 确保全局配置可以正常访问
        let global_config = ConfigLoader::get_global().expect("获取全局配置失败");
        assert_eq!(global_config.database.postgres.user, "kelisi");

        // 测试特定服务配置加载
        let mut loader = ConfigLoader::new(Component::UserServer);
        let config = loader.load().expect("加载配置失败");
        // 验证配置内容 - 这里的断言将取决于你的配置内容
        println!("加载的服务配置: {:?}", config);
    }
}
