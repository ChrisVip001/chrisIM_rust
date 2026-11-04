//! 在线路由域实现：平台在线状态、网关推送路由

use async_trait::async_trait;
use contract::error::Error;

use super::*;
use crate::traits::PresenceCache;
use crate::UserOnlineStatus;
use redis::AsyncCommands;

#[async_trait]
impl PresenceCache for RedisCache {
    /// 用户平台登录
    ///
    /// 将用户在指定平台标记为在线状态
    async fn user_platform_login(&self, user_id: &str, platform: i32) -> Result<(), Error> {
        let key = format!("{}:{}", USER_PLATFORM_ONLINE_PREFIX, user_id);
        let mut conn = self.get_connection().await?;
        conn.sadd::<_, _, ()>(&key, platform).await?;
        Ok(())
    }

    /// 用户平台登出
    ///
    /// 将用户在指定平台标记为离线状态
    async fn user_platform_logout(&self, user_id: &str, platform: i32) -> Result<(), Error> {
        let key = format!("{}:{}", USER_PLATFORM_ONLINE_PREFIX, user_id);
        let mut conn = self.get_connection().await?;
        conn.srem::<_, _, ()>(&key, platform).await?;

        // 如果用户在所有平台都下线了，删除整个集合
        let count: i64 = conn.scard(&key).await?;
        if count == 0 {
            conn.del::<_, ()>(&key).await?;
        }

        Ok(())
    }

    /// 登记用户在某平台连接所在的网关实例（推送定向路由表）
    ///
    /// HASH 结构：`user_gateway_route:{user_id}` field=平台，value=网关地址。
    /// 无 TTL：连接注销时显式移除；网关崩溃的残留条目由推送失败后的
    /// 广播兜底自愈，不需要后台清理任务。
    async fn register_user_gateway_route(&self, user_id: &str, platform: i32, gateway_addr: &str) -> Result<(), Error> {
        let key = format!("{}:{}", USER_GATEWAY_ROUTE_PREFIX, user_id);
        let mut conn = self.get_connection().await?;
        conn.hset::<_, _, _, ()>(&key, platform, gateway_addr).await?;
        Ok(())
    }

    /// 移除用户在某平台的网关路由
    async fn unregister_user_gateway_route(&self, user_id: &str, platform: i32) -> Result<(), Error> {
        let key = format!("{}:{}", USER_GATEWAY_ROUTE_PREFIX, user_id);
        let mut conn = self.get_connection().await?;
        conn.hdel::<_, _, ()>(&key, platform).await?;
        Ok(())
    }

    /// 批量查询多个用户的网关路由（pipeline 单次往返）
    async fn get_users_gateway_routes(&self, user_ids: &[String]) -> Result<Vec<Vec<String>>, Error> {
        if user_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut conn = self.get_connection().await?;
        let mut pipe = redis::pipe();
        for user_id in user_ids {
            pipe.hvals(format!("{}:{}", USER_GATEWAY_ROUTE_PREFIX, user_id));
        }
        let routes: Vec<Vec<String>> = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| Error::Internal(format!("批量查询网关路由失败: {}", e)))?;
        Ok(routes)
    }

    /// 批量获取用户完整在线状态信息
    async fn batch_get_users_online_status(&self, user_ids: &[String]) -> Result<Vec<UserOnlineStatus>, Error> {
        if user_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut conn = self.get_connection().await?;
        let mut results = Vec::with_capacity(user_ids.len());

        // 使用管道批量获取全局在线状态和平台信息
        let mut pipe = redis::pipe();

        // 添加全局在线状态检查
        for user_id in user_ids {
            pipe.sismember(USER_ONLINE_SET, user_id);
        }

        // 添加平台在线状态获取
        for user_id in user_ids {
            let key = format!("{}:{}", USER_PLATFORM_ONLINE_PREFIX, user_id);
            pipe.smembers(&key);
        }

        let batch_results: Vec<redis::Value> = pipe.query_async(&mut conn).await?;

        // 解析结果
        let user_count = user_ids.len();
        for (i, user_id) in user_ids.iter().enumerate() {
            let is_online = if let Some(redis::Value::Int(val)) = batch_results.get(i) {
                *val == 1
            } else {
                false
            };

            let online_platforms = if let Some(redis::Value::Array(platforms)) = batch_results.get(user_count + i) {
                platforms.iter()
                    .filter_map(|v| {
                        if let redis::Value::BulkString(bytes) = v {
                            String::from_utf8(bytes.clone()).ok()
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            };

            results.push(UserOnlineStatus::new(
                user_id.clone(),
                is_online,
                online_platforms,
            ));
        }

        Ok(results)
    }
}
