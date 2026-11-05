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
    ///
    /// 计数 <=0 时删掉该 field；Hash 被删空后 Redis 会自动回收键。
    async fn unread_count_set(&self, user_id: &str, conversation_id: &str, count: &i32) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 构建Redis Hash键：unread_count:user_id
        let key = format!("{}:{}", UNREAD_COUNT_PREFIX, user_id);

        if *count <= 0 {
            conn.hdel::<_, _, ()>(&key, conversation_id).await.map_err(|e| {
                Error::Internal(format!("删除用户未读消息计数失败: {}", e))
            })?;
        } else {
            conn.hset::<_, _, _, ()>(&key, conversation_id, count).await.map_err(|e| {
                Error::Internal(format!("设置用户未读消息总数失败: {}", e))
            })?;
        }

        Ok(())
    }

    /// 批量设置多个会话的未读计数
    ///
    /// 整批合成一次 pipeline，避免会话列表按会话逐个写入。
    async fn unread_count_set_batch(
        &self,
        user_id: &str,
        counts: &[(String, i32)],
    ) -> Result<(), Error> {
        if counts.is_empty() {
            return Ok(());
        }

        let mut conn = self.get_connection().await?;
        let key = format!("{}:{}", UNREAD_COUNT_PREFIX, user_id);

        let mut pipe = redis::pipe();
        for (conversation_id, count) in counts {
            // 计数 >0 写 HSET，<=0 用 HDEL 清掉该会话
            let mut cmd = if *count > 0 {
                redis::cmd("HSET")
            } else {
                redis::cmd("HDEL")
            };
            cmd.arg(&key).arg(conversation_id);
            if *count > 0 {
                cmd.arg(*count);
            }
            pipe.add_command(cmd);
        }

        pipe.query_async::<()>(&mut conn).await.map_err(|e| {
            Error::Internal(format!("批量设置用户未读消息总数失败: {}", e))
        })?;

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

        // 从Hash中删除指定会话的未读消息计数；删空后 Redis 会自动回收键
        conn.hdel::<_, _, ()>(&key, conversation_id).await.map_err(|e| {
            Error::Internal(format!("移除用户未读消息总数失败: {}", e))
        })?;

        Ok(())
    }
}
