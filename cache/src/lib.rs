/**
 * 缓存模块
 * 
 * 本模块提供缓存接口和实现，支持序列号管理、群组成员管理、
 * 注册验证码管理和用户在线状态管理等功能。
 */
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use common::message::GroupMemSeq;

use common::config::AppConfig;
use common::error::Error;

mod redis;

/// 缓存特征
/// 
/// 定义了缓存系统需要实现的所有功能接口
#[async_trait]
pub trait Cache: Sync + Send + Debug {
    /// 检查序列号是否已加载
    async fn check_seq_loaded(&self) -> Result<bool, Error>;

    /// 设置序列号已加载标志
    async fn set_seq_loaded(&self) -> Result<(), Error>;

    /// 设置接收序列号
    /// 包含：用户ID、发送最大序列号、接收最大序列号
    async fn set_seq(&self, max_seq: &[(String, i64, i64)]) -> Result<(), Error>;

    /// 设置发送序列号
    async fn set_send_seq(&self, max_seq: &[(String, i64)]) -> Result<(), Error>;

    /// 通过用户ID查询接收序列号
    async fn get_seq(&self, user_id: &str) -> Result<i64, Error>;
    
    /// 通过用户ID查询当前发送序列号和接收序列号
    async fn get_cur_seq(&self, user_id: &str) -> Result<(i64, i64), Error>;

    /// 通过用户ID查询发送序列号
    /// 返回当前发送序列号和最大发送序列号
    async fn get_send_seq(&self, user_id: &str) -> Result<(i64, i64), Error>;

    /// 增加用户的接收序列号
    async fn increase_seq(&self, user_id: &str) -> Result<(i64, i64, bool), Error>;

    /// 增加用户的发送序列号
    async fn incr_send_seq(&self, user_id: &str) -> Result<(i64, i64, bool), Error>;

    /// 增加群组成员序列号
    async fn incr_group_seq(&self, members: Vec<String>) -> Result<Vec<GroupMemSeq>, Error>;

    /// 查询群组成员ID
    async fn query_group_members_id(&self, group_id: &str) -> Result<Vec<String>, Error>;

    /// 保存群组成员ID，通常在创建群组时调用
    async fn save_group_members_id(
        &self,
        group_id: &str,
        members_id: Vec<String>,
    ) -> Result<(), Error>;

    /// 添加一个成员ID到群组成员集合
    async fn add_group_member_id(&self, member_id: &str, group_id: &str) -> Result<(), Error>;

    /// 从群组成员集合中移除成员ID
    async fn remove_group_member_id(&self, group_id: &str, member_id: &str) -> Result<(), Error>;

    /// 批量从群组中移除成员
    async fn remove_group_member_batch(
        &self,
        group_id: &str,
        member_id: &[&str],
    ) -> Result<(), Error>;

    /// 删除群组所有成员
    async fn del_group_members(&self, group_id: &str) -> Result<(), Error>;

    /// 保存注册验证码
    async fn save_register_code(&self, email: &str, code: &str) -> Result<(), Error>;

    /// 获取注册验证码
    async fn get_register_code(&self, email: &str) -> Result<Option<String>, Error>;

    /// 用户注册后删除注册验证码
    async fn del_register_code(&self, email: &str) -> Result<(), Error>;

    /// 用户登录
    async fn user_login(&self, user_id: &str) -> Result<(), Error>;

    /// 用户登出
    async fn user_logout(&self, user_id: &str) -> Result<(), Error>;

    /// 在线用户计数
    async fn online_count(&self) -> Result<i64, Error>;
}

/// 根据配置创建缓存实例
///
/// # 参数
/// * `config` - 应用配置
///
/// # 返回
/// * 实现了Cache特征的实例，被Arc包裹以便共享
pub fn cache(config: &AppConfig) -> Arc<dyn Cache> {
    Arc::new(redis::RedisCache::from_config(config))
}
