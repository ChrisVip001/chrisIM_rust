//! 未读域实现：会话未读计数（Hash 存储，Lua 保证减计数的原子性）

use async_trait::async_trait;
use contract::error::Error;

use super::*;
use crate::traits::UnreadCache;
use redis::AsyncCommands;

#[async_trait]
impl UnreadCache for RedisCache {
    /// 获取用户在指定会话中的未读消息总数
    ///
    /// 使用Hash结构存储，一个用户一个Hash，每个会话作为field
    async fn unread_count_sum(&self, user_id: &str, conversation_id: &str) -> Result<i32, Error> {
        let mut conn = self.get_connection().await?;

        // 构建Redis Hash键：unread_count:user_id
        let key = format!("{}:{}", UNREAD_COUNT_PREFIX, user_id);

        // 从Hash中获取指定会话的未读消息数量
        let count: Option<i32> = conn.hget(&key, conversation_id).await.map_err(|e| {
            Error::Internal(format!("获取用户未读消息总数失败: {}", e))
        })?;

        Ok(count.unwrap_or(0))
    }

    /// 设置用户在指定会话中的未读消息总数
    async fn unread_count_set(&self, user_id: &str, conversation_id: &str, count: &i32) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 构建Redis Hash键：unread_count:user_id
        let key = format!("{}:{}", UNREAD_COUNT_PREFIX, user_id);

        if *count <= 0 {
            // 如果计数为0或负数，从Hash中删除该会话的field
            conn.hdel::<_, _, ()>(&key, conversation_id).await.map_err(|e| {
                Error::Internal(format!("删除用户未读消息计数失败: {}", e))
            })?;

            // 检查Hash是否为空，如果为空则删除整个Hash
            let hash_size: i64 = conn.hlen(&key).await.map_err(|e| {
                Error::Internal(format!("检查Hash大小失败: {}", e))
            })?;

            if hash_size == 0 {
                conn.del::<_, ()>(&key).await.map_err(|e| {
                    Error::Internal(format!("删除空Hash失败: {}", e))
                })?;
            }
        } else {
            // 在Hash中设置指定会话的未读消息数量
            conn.hset::<_, _, _, ()>(&key, conversation_id, count).await.map_err(|e| {
                Error::Internal(format!("设置用户未读消息总数失败: {}", e))
            })?;
        }

        Ok(())
    }

    /// 减少用户在指定会话中的未读消息总数（减1）
    ///
    /// 当用户阅读消息时调用此方法，将未读计数减1
    /// 如果计数减到0或以下，会自动删除该键
    async fn unread_count_decrement(&self, user_id: &str, conversation_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 构建Redis Hash键：unread_count:user_id
        let key = format!("{}:{}", UNREAD_COUNT_PREFIX, user_id);

        // 使用Lua脚本原子性地减少Hash中指定field的计数
        let script = r#"
            local key = KEYS[1]
            local field = ARGV[1]
            local current = redis.call('HGET', key, field)

            if current == false then
                -- field不存在，返回0
                return 0
            end

            local count = tonumber(current)
            if count <= 1 then
                -- 计数为1或更少，删除field
                redis.call('HDEL', key, field)

                -- 检查Hash是否为空，如果为空则删除整个Hash
                local hash_size = redis.call('HLEN', key)
                if hash_size == 0 then
                    redis.call('DEL', key)
                end
                return 0
            else
                -- 减少计数
                local new_count = count - 1
                redis.call('HSET', key, field, new_count)
                return new_count
            end
        "#;

        let _new_count: i32 = redis::Script::new(script)
            .key(&key)
            .arg(conversation_id)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| {
                Error::Internal(format!("减少用户未读消息总数失败: {}", e))
            })?;

        Ok(())
    }

    /// 移除用户在指定会话中的未读消息计数
    async fn unread_count_remove(&self, user_id: &str, conversation_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 构建Redis Hash键：unread_count:user_id
        let key = format!("{}:{}", UNREAD_COUNT_PREFIX, user_id);

        // 从Hash中删除指定会话的未读消息计数
        conn.hdel::<_, _, ()>(&key, conversation_id).await.map_err(|e| {
            Error::Internal(format!("移除用户未读消息总数失败: {}", e))
        })?;

        // 检查Hash是否为空，如果为空则删除整个Hash
        let hash_size: i64 = conn.hlen(&key).await.map_err(|e| {
            Error::Internal(format!("检查Hash大小失败: {}", e))
        })?;

        if hash_size == 0 {
            conn.del::<_, ()>(&key).await.map_err(|e| {
                Error::Internal(format!("删除空Hash失败: {}", e))
            })?;
        }

        Ok(())
    }
}
