/**
 * 缓存模块
 *
 * 本模块提供缓存接口和实现，支持序列号管理、群组成员管理、
 * 注册验证码管理和用户在线状态管理等功能。
 *
 * 接口层按业务域拆分为领域 trait（见 [`traits`]）：
 * SeqCache / SessionCache / RelationCache / PresenceCache / UnreadCache /
 * ConversationIndexCache，[`Cache`] 作为组合入口保持对历史调用方的兼容。
 * 新代码建议依赖更窄的领域 trait，让依赖关系表达真实的业务边界。
 */
use std::sync::Arc;

use runtime::config::AppConfig;

mod redis;
mod traits;

pub use redis::RedisCache;
pub use traits::{
    BlacklistCheckResult, Cache, ConversationIndexCache, PresenceCache, RelationCache, SeqCache,
    SessionCache, UnreadCache, UserOnlineStatus,
};

/// 根据配置创建缓存实例
///
/// 返回具体类型 `Arc<RedisCache>`：调用方按各自的真实依赖
/// 收窄为领域 trait（`Arc<dyn SeqCache>` 等），让构造签名表达业务边界；
/// 同一实例可同时收窄为多个领域 trait（引用计数克隆，零成本）。
///
/// # 参数
/// * `config` - 应用配置
///
/// # 返回
/// 被 Arc 包裹的具体缓存实例
pub async fn cache(config: &AppConfig) -> Arc<RedisCache> {
    Arc::new(redis::RedisCache::from_config(config).await)
}
