//! 缓存领域接口
//!
//! 此前 [`Cache`] 是一个 47 个方法的"上帝接口"：序列号、群成员、
//! 好友黑名单、令牌验证码、在线路由、未读计数混在同一个 trait 里，
//! 任何实现方都必须全量实现，调用方也无法表达"我只需要序号能力"。
//!
//! 现按业务域拆分为五个小 trait，[`Cache`] 作为组合入口保持不变——
//! 现有 `Arc<dyn Cache>` 调用方零改动；新代码应依赖更窄的领域 trait，
//! 让依赖关系表达真实的业务边界。
//!
//! | 领域 trait | 职责 |
//! |---|---|
//! | [`SeqCache`] | 用户/群组/会话的序号与号段分配 |
//! | [`SessionCache`] | 登录令牌与注册验证码 |
//! | [`RelationCache`] | 好友、黑名单、群成员关系 |
//! | [`PresenceCache`] | 在线状态与网关推送路由 |
//! | [`UnreadCache`] | 会话未读计数 |
//! | [`ConversationIndexCache`] | 会话列表的活跃时间索引 |

use async_trait::async_trait;
use contract::error::Error;
use contract::proto::message::GroupMemSeq;
use std::fmt::Debug;

/// 序列号域：接收/发送序号、会话号段、群成员批量序号
#[async_trait]
pub trait SeqCache: Sync + Send + Debug {
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

    /// 用持久化水位初始化会话号段（仅当键不存在时生效）
    ///
    /// 返回本次预留的号段上界。调用方必须把它持久化，
    /// 否则 Redis 丢失后从旧水位重建会重复分配已用过的序号。
    async fn init_conversation_seq(
        &self,
        conversation_id: &str,
        base_seq: i64,
    ) -> Result<i64, Error>;

    /// 领取会话下一个序号
    ///
    /// 返回 `(当前序号, 号段上界, 是否需要回写水位)`。
    /// `当前序号 == 0` 表示号段尚未初始化，调用方需先用 `init_conversation_seq`
    /// 以 PG 水位冷启动后重试；存在性检查已并入脚本原子执行，热路径仅一次往返。
    /// `是否需要回写` 为真时表示已进入新号段，调用方应把上界持久化，
    /// 这样 Redis 丢失后可据此重建，不会重复分配。
    async fn incr_conversation_seq(
        &self,
        conversation_id: &str,
    ) -> Result<(i64, i64, bool), Error>;

    /// 增加群组成员序列号
    async fn incr_group_seq(&self, members: Vec<String>) -> Result<Vec<GroupMemSeq>, Error>;
}

/// 会话域：登录令牌、注册验证码
#[async_trait]
pub trait SessionCache: Sync + Send + Debug {
    /// 存储指定平台的访问令牌
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `token` - 访问令牌
    /// * `platform` - 平台类型
    /// * `expiry_seconds` - 过期时间（秒）
    async fn save_access_token_for_platform(&self, user_id: &str, token: &str, platform: i32, expiry_seconds: u64) -> Result<(), Error>;

    /// 存储指定平台的刷新令牌
    async fn save_refresh_token_for_platform(&self, user_id: &str, token: &str, platform: i32, expiry_seconds: u64) -> Result<(), Error>;

    /// 获取指定平台的访问令牌
    ///
    /// # 返回值
    /// * `Option<String>` - 令牌，如果不存在或已过期则返回None
    async fn get_access_token_for_platform(&self, user_id: &str, platform: i32) -> Result<Option<String>, Error>;

    /// 获取指定平台的刷新令牌
    async fn get_refresh_token_for_platform(&self, user_id: &str, platform: i32) -> Result<Option<String>, Error>;

    /// 删除指定平台的访问令牌
    async fn delete_access_token_for_platform(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 删除指定平台的刷新令牌
    async fn delete_refresh_token_for_platform(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 保存注册验证码
    async fn save_register_code(&self, email: &str, code: &str) -> Result<(), Error>;

    /// 获取注册验证码
    async fn get_register_code(&self, email: &str) -> Result<Option<String>, Error>;

    /// 用户注册后删除注册验证码
    async fn del_register_code(&self, email: &str) -> Result<(), Error>;
}

/// 关系域：好友关系、黑名单、群成员
#[async_trait]
pub trait RelationCache: Sync + Send + Debug {
    /// 保存双向好友关系
    ///
    /// 在缓存中存储两个用户之间的双向好友关系，便于快速查询
    async fn save_bidirectional_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Error>;

    /// 检查双向好友关系是否存在
    ///
    /// # 返回值
    /// * `bool` - 如果存在好友关系返回true，否则返回false
    async fn check_friendship_exists(&self, user_id: &str, friend_id: &str) -> Result<bool, Error>;

    /// 删除双向好友关系
    async fn delete_bidirectional_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Error>;

    /// 获取用户的所有好友ID
    ///
    /// # 返回值
    /// * `Vec<String>` - 好友ID列表
    async fn get_all_friend_ids(&self, user_id: &str) -> Result<Vec<String>, Error>;

    /// 添加用户黑名单关系
    async fn add_user_to_blacklist(&self, user_id: &str, blocked_user_id: &str) -> Result<(), Error>;

    /// 从黑名单中移除用户
    async fn remove_user_from_blacklist(&self, user_id: &str, blocked_user_id: &str) -> Result<(), Error>;

    /// 检查用户是否在黑名单中
    async fn is_user_in_blacklist(&self, user_id: &str, target_user_id: &str) -> Result<bool, Error>;

    /// 获取用户的黑名单列表
    async fn get_user_blacklist(&self, user_id: &str) -> Result<Vec<String>, Error>;

    /// 双向检查黑名单关系
    async fn check_bidirectional_blacklist(&self, user_id1: &str, user_id2: &str) -> Result<BlacklistCheckResult, Error>;

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
}

/// 在线路由域：平台在线状态、网关推送路由
#[async_trait]
pub trait PresenceCache: Sync + Send + Debug {
    /// 用户平台登录
    ///
    /// 将用户在指定平台标记为在线状态
    async fn user_platform_login(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 用户平台登出
    ///
    /// 将用户在指定平台标记为离线状态
    async fn user_platform_logout(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 登记用户在某平台连接所在的网关实例（推送定向路由表）
    ///
    /// 网关在连接注册时调用；推送方据此把消息直接发到目标网关，
    /// 而不是广播到全部实例。路由条目无 TTL，连接注销时移除；
    /// 网关崩溃留下的残留条目由推送失败后的广播兜底自愈。
    async fn register_user_gateway_route(&self, user_id: &str, platform: i32, gateway_addr: &str) -> Result<(), Error>;

    /// 移除用户在某平台的网关路由
    async fn unregister_user_gateway_route(&self, user_id: &str, platform: i32) -> Result<(), Error>;

    /// 批量查询多个用户的网关路由
    ///
    /// 返回与输入等长的集合列表，每个集合是该用户全部平台所在的
    /// 去重网关地址；用户完全离线时对应集合为空。
    /// 使用 pipeline 一次往返，供群消息推送按网关分组。
    async fn get_users_gateway_routes(&self, user_ids: &[String]) -> Result<Vec<Vec<String>>, Error>;

    /// 批量获取用户完整在线状态信息
    async fn batch_get_users_online_status(&self, user_ids: &[String]) -> Result<Vec<UserOnlineStatus>, Error>;
}

/// 未读域：会话未读计数
#[async_trait]
pub trait UnreadCache: Sync + Send + Debug {
    /// 获取用户在指定会话中的未读消息总数
    async fn unread_count_sum(&self, user_id: &str, conversation_id: &str) -> Result<i32, Error>;

    /// 设置用户在指定会话中的未读消息总数
    async fn unread_count_set(&self, user_id: &str, conversation_id: &str, count: &i32) -> Result<(), Error>;

    /// 减少用户在指定会话中的未读消息总数（减1）
    async fn unread_count_decrement(&self, user_id: &str, conversation_id: &str) -> Result<(), Error>;

    /// 移除用户在指定会话中的未读消息计数
    async fn unread_count_remove(&self, user_id: &str, conversation_id: &str) -> Result<(), Error>;
}

/// 会话列表域：按最近活跃时间排序的会话索引
///
/// 索引只是排序与截断的依据，权威数据仍在 `user_conversation` / `conversation` 表；
/// 索引缺失时调用方回落到数据库查询并回填。
#[async_trait]
pub trait ConversationIndexCache: Sync + Send + Debug {
    /// 把会话推进到这些成员的列表索引
    ///
    /// `entries` 为 `(user_id, conversation_id, active_time_ms)`，
    /// 同一个 `(user_id, conversation_id)` 出现多次时保留最大时间。
    /// 分数只增不减，重复投递不会把会话往后挪。
    async fn touch_conversation_index(
        &self,
        entries: &[(String, String, i64)],
    ) -> Result<(), Error>;

    /// 按活跃时间倒序返回 `limit` 个会话及其时间（毫秒）
    async fn recent_conversations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, Error>;
}

/// 缓存组合入口
///
/// 领域 trait 的超集。历史调用方 `Arc<dyn Cache>` 无需改动；
/// 新代码建议只依赖实际需要的领域 trait（如 `Arc<dyn SeqCache>`），
/// 让接口依赖与业务边界保持一致。
#[async_trait]
pub trait Cache:
    SeqCache + SessionCache + RelationCache + PresenceCache + UnreadCache + ConversationIndexCache
{
}

/// 自动为任何实现了全部领域 trait 的类型实现组合入口
#[async_trait]
impl<T> Cache for T where
    T: SeqCache
        + SessionCache
        + RelationCache
        + PresenceCache
        + UnreadCache
        + ConversationIndexCache
        + Send
        + Sync
        + Debug
{
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
