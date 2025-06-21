/**
 * 缓存模块
 * 
 * 本模块提供缓存接口和实现，支持序列号管理、群组成员管理、
 * 注册验证码管理和用户在线状态管理等功能。
 */
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use common::proto::message::{GroupMemSeq, PlatformType};
use serde::{Deserialize, Serialize};

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
    
    
    /// 用户平台登录
    /// 
    /// 将用户在指定平台标记为在线状态
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `platform` - 平台类型
    async fn user_platform_login(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 用户平台登出
    /// 
    /// 将用户在指定平台标记为离线状态
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `platform` - 平台类型
    async fn user_platform_logout(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 存储指定平台的访问令牌
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `token` - 访问令牌
    /// * `platform` - 平台类型
    /// * `expiry_seconds` - 过期时间（秒）
    async fn save_access_token_for_platform(&self, user_id: &str, token: &str, platform: i32, expiry_seconds: u64) -> Result<(), Error>;

    /// 存储指定平台的刷新令牌
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `token` - 刷新令牌
    /// * `platform` - 平台类型
    /// * `expiry_seconds` - 过期时间（秒）
    async fn save_refresh_token_for_platform(&self, user_id: &str, token: &str, platform: i32, expiry_seconds: u64) -> Result<(), Error>;

    /// 获取指定平台的访问令牌
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `platform` - 平台类型
    /// 
    /// # 返回值
    /// * `Option<String>` - 令牌，如果不存在或已过期则返回None
    async fn get_access_token_for_platform(&self, user_id: &str, platform: i32) -> Result<Option<String>, Error>;

    /// 获取指定平台的刷新令牌
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `platform` - 平台类型
    /// 
    /// # 返回值
    /// * `Option<String>` - 令牌，如果不存在或已过期则返回None
    async fn get_refresh_token_for_platform(&self, user_id: &str, platform: i32) -> Result<Option<String>, Error>;

    /// 删除指定平台的访问令牌
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `platform` - 平台类型
    async fn delete_access_token_for_platform(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 删除指定平台的刷新令牌
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `platform` - 平台类型
    async fn delete_refresh_token_for_platform(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 批量获取用户完整在线状态信息
    /// 
    /// # 参数
    /// * `user_ids` - 用户ID列表
    /// 
    /// # 返回值
    /// * `Vec<UserOnlineStatus>` - 用户在线状态信息列表
    async fn batch_get_users_online_status(&self, user_ids: &[String]) -> Result<Vec<UserOnlineStatus>, Error>;
    
    /// 保存双向好友关系
    /// 
    /// 在缓存中存储两个用户之间的双向好友关系，便于快速查询
    /// 
    /// # 参数
    /// * `user_id` - 第一个用户的ID
    /// * `friend_id` - 第二个用户的ID
    async fn save_bidirectional_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Error>;

    /// 检查双向好友关系是否存在
    /// 
    /// 快速检查两个用户之间是否存在好友关系
    /// 
    /// # 参数
    /// * `user_id` - 第一个用户的ID
    /// * `friend_id` - 第二个用户的ID
    /// 
    /// # 返回值
    /// * `bool` - 如果存在好友关系返回true，否则返回false
    async fn check_friendship_exists(&self, user_id: &str, friend_id: &str) -> Result<bool, Error>;

    /// 删除双向好友关系
    /// 
    /// 从缓存中删除两个用户之间的双向好友关系
    /// 
    /// # 参数
    /// * `user_id` - 第一个用户的ID
    /// * `friend_id` - 第二个用户的ID
    async fn delete_bidirectional_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Error>;

    /// 获取用户的所有好友ID
    /// 
    /// 从缓存中获取指定用户的所有好友ID列表
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// 
    /// # 返回值
    /// * `Vec<String>` - 好友ID列表
    async fn get_all_friend_ids(&self, user_id: &str) -> Result<Vec<String>, Error>;
    
    /// 添加用户黑名单关系
    /// 
    /// 将指定用户添加到另一用户的黑名单中
    /// 
    /// # 参数
    /// * `user_id` - 拉黑操作的发起用户ID
    /// * `blocked_user_id` - 被拉黑的用户ID
    async fn add_user_to_blacklist(&self, user_id: &str, blocked_user_id: &str) -> Result<(), Error>;
    
    /// 从黑名单中移除用户
    /// 
    /// 将指定用户从另一用户的黑名单中移除
    /// 
    /// # 参数
    /// * `user_id` - 解除拉黑操作的发起用户ID
    /// * `blocked_user_id` - 被解除拉黑的用户ID
    async fn remove_user_from_blacklist(&self, user_id: &str, blocked_user_id: &str) -> Result<(), Error>;
    
    /// 检查用户是否在黑名单中
    /// 
    /// 检查一个用户是否被另一用户拉黑
    /// 
    /// # 参数
    /// * `user_id` - 可能拉黑他人的用户ID
    /// * `target_user_id` - 可能被拉黑的用户ID
    /// 
    /// # 返回值
    /// * `bool` - 如果目标用户被拉黑则返回true，否则返回false
    async fn is_user_in_blacklist(&self, user_id: &str, target_user_id: &str) -> Result<bool, Error>;

    // 双向检查黑名单
    
    /// 获取用户的黑名单列表
    /// 
    /// 获取指定用户拉黑的所有用户ID列表
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// 
    /// # 返回值
    /// * `Vec<String>` - 被拉黑的用户ID列表
    async fn get_user_blacklist(&self, user_id: &str) -> Result<Vec<String>, Error>;
    
    /// 双向检查黑名单关系
    /// 
    /// 检查两个用户之间是否存在任意方向的黑名单关系
    /// 
    /// # 参数
    /// * `user_id1` - 第一个用户的ID
    /// * `user_id2` - 第二个用户的ID
    /// 
    /// # 返回值
    /// * `BlacklistCheckResult` - 黑名单检查结果，包含是否存在黑名单关系及关系方向
    async fn check_bidirectional_blacklist(&self, user_id1: &str, user_id2: &str) -> Result<BlacklistCheckResult, Error>;
}

/// 用户在线状态信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserOnlineStatus {
    /// 用户ID
    pub user_id: String,
    /// 是否在线（全局状态）
    pub is_online: bool,
    /// 在线平台列表
    pub online_platforms: Vec<String>,
    /// 在线平台数量
    pub platform_count: i64,
}

impl UserOnlineStatus {
    /// 创建新的用户在线状态
    pub fn new(user_id: String, is_online: bool, online_platforms: Vec<String>) -> Self {
        let platform_count = online_platforms.len() as i64;
        Self {
            user_id,
            is_online,
            online_platforms,
            platform_count,
        }
    }
}

/// 黑名单检查结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlacklistCheckResult {
    /// 是否存在黑名单关系
    pub has_blacklist: bool,
    /// 用户1是否将用户2加入黑名单
    pub user1_blocked_user2: bool,
    /// 用户2是否将用户1加入黑名单
    pub user2_blocked_user1: bool,
}

impl BlacklistCheckResult {
    /// 创建新的黑名单检查结果
    pub fn new(user1_blocked_user2: bool, user2_blocked_user1: bool) -> Self {
        let has_blacklist = user1_blocked_user2 || user2_blocked_user1;
        Self {
            has_blacklist,
            user1_blocked_user2,
            user2_blocked_user1,
        }
    }
}

/// 根据配置创建缓存实例
///
/// # 参数
/// * `config` - 应用配置
///
/// # 返回
/// * 实现了Cache特征的实例，被Arc包裹以便共享
pub async fn cache(config: &AppConfig) -> Arc<dyn Cache> {
    Arc::new(redis::RedisCache::from_config(config).await)
}
