// 导入必要的依赖
use async_trait::async_trait;
use std::collections::HashSet;
use std::net::SocketAddr;
use crate::Error;

/// 服务获取器特征
///
/// 定义了从服务注册中心获取服务地址的接口
/// 实现此特征的类型可以用于服务发现过程
#[async_trait]
pub trait ServiceFetcher: Send + Sync {
    /// 获取服务地址集合
    ///
    /// 从服务注册中心获取服务地址列表
    /// 
    /// # 返回
    /// 返回一个包含服务套接字地址的集合，如果发生错误则返回 Error
    async fn fetch(&self) -> Result<HashSet<SocketAddr>, Error>;
}
