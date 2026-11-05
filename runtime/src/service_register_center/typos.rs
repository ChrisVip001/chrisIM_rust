// 导入 serde 用于序列化和反序列化
use serde::{Deserialize, Serialize};

/// 服务注册信息
///
/// 包含向服务注册中心注册服务所需的所有信息
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Registration {
    /// 服务实例的唯一标识
    pub id: String,
    /// 服务名称
    pub name: String,
    /// 服务主机地址
    pub host: String,
    /// 服务端口号
    pub port: u16,
    /// 服务标签，用于分类和过滤
    pub tags: Vec<String>,
    /// 健康检查配置
    pub check: Option<HealthCheck>,
}

/// 健康检查配置
///
/// 定义服务注册中心如何检查服务的健康状态
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct HealthCheck {
    /// 健康检查类型
    pub health_type: String,
    /// 健康检查名称
    pub name: String,
    /// 健康检查的URL
    pub url: String,
    /// 健康检查的时间间隔
    pub interval: String,
    /// 健康检查的超时时间
    pub timeout: String,
    /// 服务不健康后多久取消注册
    pub deregister_after: String,
}

impl HealthCheck {
    /// 配置段 → 注册中心健康检查定义的唯一转换点
    ///
    /// 注册入口（gRPC 服务、WebSocket 接入层）共用，避免字段映射漂移。
    pub fn from_config(cfg: &crate::config::HealthCheckConfig) -> Self {
        Self {
            health_type: cfg.health_type.clone(),
            name: cfg.name.clone(),
            url: cfg.url.clone(),
            interval: cfg.interval.to_string(),
            timeout: cfg.timeout.to_string(),
            deregister_after: cfg.deregister_after.to_string(),
        }
    }
}


/// 服务类型枚举，用于确定健康检查方式
#[derive(Debug, Clone, PartialEq, Default)]
pub enum HealthType {
    /// HTTP服务，使用HTTP健康检查
    #[default]
    Http,
    /// gRPC服务，使用TTL健康检查
    Grpc
}

impl HealthType {
    /// 确定健康检查类型
    pub fn determine_health_type(health_type: String) -> HealthType {
        match health_type.to_lowercase().as_str() {
            "http" => HealthType::Http,
            "grpc" => HealthType::Grpc,
            _ => HealthType::Http,
        }
    }
}

/// 已发现的服务实例信息
///
/// 从服务注册中心获取的服务实例详细信息
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Service {
    /// 服务实例的唯一标识
    #[serde(rename = "ID")]
    pub id: String,
    /// 服务名称
    #[serde(rename = "Service")]
    pub service: String,
    /// 服务地址
    #[serde(rename = "Address")]
    pub address: String,
    /// 服务端口
    #[serde(rename = "Port")]
    pub port: u16,
    /// 服务标签
    #[serde(rename = "Tags")]
    pub tags: Vec<String>,
    /// 数据中心
    #[serde(rename = "Datacenter")]
    pub datacenter: String,
}
