use common::{Error, Result};
use redis::{AsyncCommands, aio::MultiplexedConnection};
use tracing::{error, debug};

/// 令牌仓库，负责令牌的存储和检索
pub struct TokenRepository {
    /// 1.redis::aio::MultiplexedConnection是专门为异步I/O设计的，而此项目使用Tokio异步运行时。
    /// 使用MultiplexedConnection允许在不阻塞整个线程的情况下执行Redis操作，这对于高并发的微服务架构至关重要。
    /// 2.MultiplexedConnection名称中的"Multiplexed"意味着它能够在单个TCP连接上复用多个Redis命令。
    /// 3.MultiplexedConnection是线程安全的，可以被克隆并在多个任务之间共享，这意味着它可以在不同的异步任务中并发使用，而不需要额外的同步机制
    /// 4.在微服务架构中，尤其是使用gRPC等异步API的项目里，所有组件都应该是非阻塞的。使用MultiplexedConnection确保Redis操作不会成为系统中的性能瓶颈。
    redis: MultiplexedConnection,
}

impl TokenRepository {
    pub fn new(redis: MultiplexedConnection) -> Self {
        Self { redis }
    }
    
    /// 存储访问令牌
    pub async fn store_access_token(&self, user_id: &str, token: &str, expires_in: i64) -> Result<()> {
        // 存储访问令牌，键为 access_token:{token}，值为用户ID
        let mut conn = self.redis.clone();
        let token_key = format!("access_token:{}", token);
        let user_tokens_key = format!("user_tokens:{}", user_id);
        
        // 设置令牌 -> 用户ID 的映射，带过期时间
        if let Err(err) = conn.set_ex::<_, _, ()>(&token_key, user_id, expires_in as u64).await {
            error!("存储访问令牌失败: {}", err);
            return Err(Error::Redis(err));
        }
        
        // 添加到用户的令牌集合中，便于查询和注销
        match conn.sadd::<_, _, i32>(&user_tokens_key, &token).await {
            Ok(_) => debug!("将令牌添加到用户集合成功"),
            Err(err) => error!("将令牌添加到用户集合失败: {}", err),
        }
        
        Ok(())
    }
    
    /// 存储刷新令牌
    pub async fn store_refresh_token(&self, user_id: &str, token: &str, expires_in: i64) -> Result<()> {
        // 存储刷新令牌，键为 refresh_token:{token}，值为用户ID
        let mut conn = self.redis.clone();
        let token_key = format!("refresh_token:{}", token);
        
        // 设置令牌 -> 用户ID 的映射，带过期时间
        if let Err(err) = conn.set_ex::<_, _, ()>(&token_key, user_id, expires_in as u64).await {
            error!("存储刷新令牌失败: {}", err);
            return Err(Error::Redis(err));
        }
        
        Ok(())
    }
    
    /// 验证访问令牌
    pub async fn validate_access_token(&self, token: &str) -> Result<Option<String>> {
        let mut conn = self.redis.clone();
        let token_key = format!("access_token:{}", token);
        
        match conn.get::<_, Option<String>>(&token_key).await {
            Ok(Some(user_id)) => {
                debug!("令牌有效，用户ID: {}", user_id);
                Ok(Some(user_id))
            },
            Ok(None) => {
                debug!("令牌不存在或已过期");
                Ok(None)
            },
            Err(err) => {
                error!("验证令牌时发生Redis错误: {}", err);
                Err(Error::Redis(err))
            }
        }
    }
    
    /// 验证刷新令牌
    pub async fn validate_refresh_token(&self, token: &str) -> Result<Option<String>> {
        let mut conn = self.redis.clone();
        let token_key = format!("refresh_token:{}", token);
        
        match conn.get::<_, Option<String>>(&token_key).await {
            Ok(Some(user_id)) => {
                debug!("刷新令牌有效，用户ID: {}", user_id);
                Ok(Some(user_id))
            },
            Ok(None) => {
                debug!("刷新令牌不存在或已过期");
                Ok(None)
            },
            Err(err) => {
                error!("验证刷新令牌时发生Redis错误: {}", err);
                Err(Error::Redis(err))
            }
        }
    }
    
    /// 使令牌失效
    pub async fn invalidate_token(&self, token: &str) -> Result<bool> {
        let mut conn = self.redis.clone();
        let access_token_key = format!("access_token:{}", token);
        
        // 首先获取用户ID
        let user_id: Option<String> = match conn.get(&access_token_key).await {
            Ok(id) => id,
            Err(err) => {
                error!("从令牌获取用户ID失败: {}", err);
                return Err(Error::Redis(err));
            }
        };
        
        if let Some(user_id) = user_id {
            // 从用户的令牌集合中移除
            let user_tokens_key = format!("user_tokens:{}", user_id);
            match conn.srem::<_, _, i32>(&user_tokens_key, token).await {
                Ok(_) => debug!("从用户集合中移除令牌成功"),
                Err(err) => error!("从用户集合中移除令牌失败: {}", err),
            }
        }
        
        // 删除令牌
        match conn.del::<_, i32>(&access_token_key).await {
            Ok(1) => {
                debug!("令牌已成功失效");
                Ok(true)
            },
            Ok(_) => {
                debug!("令牌不存在或已失效");
                Ok(false)
            },
            Err(err) => {
                error!("使令牌失效时发生Redis错误: {}", err);
                Err(Error::Redis(err))
            }
        }
    }
    
    /// 使用户的所有令牌失效
    #[warn(dead_code)]
    pub async fn invalidate_user_tokens(&self, user_id: &str) -> Result<i32> {
        let mut conn = self.redis.clone();
        let user_tokens_key = format!("user_tokens:{}", user_id);
        
        // 获取用户的所有令牌
        let tokens: Vec<String> = match conn.smembers(&user_tokens_key).await {
            Ok(tokens) => tokens,
            Err(err) => {
                error!("获取用户令牌集合失败: {}", err);
                return Err(Error::Redis(err));
            }
        };
        
        let mut invalidated_count = 0;
        
        // 逐个删除令牌
        for token in tokens {
            let token_key = format!("access_token:{}", token);
            match conn.del::<_, i32>(&token_key).await {
                Ok(1) => {
                    invalidated_count += 1;
                    debug!("令牌 {} 已失效", token);
                },
                Ok(_) => debug!("令牌 {} 不存在或已失效", token),
                Err(err) => error!("使令牌 {} 失效时发生Redis错误: {}", token, err),
            }
        }
        
        // 清空用户的令牌集合
        match conn.del::<_, i32>(&user_tokens_key).await {
            Ok(_) => debug!("用户令牌集合已清空"),
            Err(err) => error!("清空用户令牌集合失败: {}", err),
        }
        
        Ok(invalidated_count)
    }
} 