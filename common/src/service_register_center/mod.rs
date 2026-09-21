// 导入标准库和必要的依赖
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use crate::config::AppConfig;
use crate::Error;

// 声明子模块
pub mod consul;
pub mod typos;

// 导入类型定义
pub use crate::service_register_center::typos::Registration;
pub use crate::service_register_center::consul::Consul;

/// 服务注册与发现接口
///
/// 定义了服务注册和注销的核心功能
#[async_trait]
pub trait ServiceRegister: Send + Sync + Debug {
    /// 向注册中心注册服务
    ///
    /// # 参数
    /// * `registration` - 包含服务信息的注册对象
    ///
    /// # 返回
    /// 成功返回 Ok(()), 失败返回 Error
    async fn register(&self, registration: Registration) -> Result<String, Error>;

    /// 从注册中心注销服务
    ///
    /// # 参数
    /// * `service_id` - 服务的唯一标识
    ///
    /// # 返回
    /// 成功返回 Ok(()), 失败返回 Error
    async fn deregister(&self, service_id: &str) -> Result<(), Error>;
 
    /// 从注册中心获取服务列表
    /// 
    /// # 参数
    /// * `service_name` - 服务名称
    /// 
    /// # 返回
    /// 返回一个包含服务信息的 HashMap，键为服务 ID，值为服务信息
    async fn find_by_name(&self, service_name: &str) -> Result<HashMap<String, Registration>, Error>;
}

/// 创建服务注册中心实例
///
/// 根据配置创建服务注册中心的具体实现
///
/// # 参数
/// * `config` - 应用配置对象
///
/// # 返回
/// 返回一个实现了 ServiceRegister 特征的 Arc 包装对象
pub fn service_register_center(config: &AppConfig) -> Arc<dyn ServiceRegister> {
    Arc::new(Consul::from_config(config))
}
