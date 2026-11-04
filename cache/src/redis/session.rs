//! 会话域实现：平台令牌、注册验证码

use async_trait::async_trait;
use contract::error::Error;

use super::*;
use crate::traits::SessionCache;
use redis::AsyncCommands;

#[async_trait]
impl SessionCache for RedisCache {
    /// 存储指定平台的访问令牌
    async fn save_access_token_for_platform(&self, user_id: &str, token: &str, platform: i32, expiry_seconds: u64) -> Result<(), Error> {
        let key = format!("token:access:{}:{}", user_id, platform);
        let mut conn = self.get_connection().await?;
        conn.set_ex::<_, _, ()>(&key, token, expiry_seconds).await?;
        Ok(())
    }

    /// 存储指定平台的刷新令牌
    async fn save_refresh_token_for_platform(&self, user_id: &str, token: &str, platform: i32, expiry_seconds: u64) -> Result<(), Error> {
        let key = format!("token:refresh:{}:{}", user_id, platform);
        let mut conn = self.get_connection().await?;
        conn.set_ex::<_, _, ()>(&key, token, expiry_seconds).await?;
        Ok(())
    }

    /// 获取指定平台的访问令牌
    async fn get_access_token_for_platform(&self, user_id: &str, platform: i32) -> Result<Option<String>, Error> {
        let key = format!("token:access:{}:{}", user_id, platform);
        let mut conn = self.get_connection().await?;
        let result: Option<String> = conn.get(&key).await?;
        Ok(result)
    }

    /// 获取指定平台的刷新令牌
    async fn get_refresh_token_for_platform(&self, user_id: &str, platform: i32) -> Result<Option<String>, Error> {
        let key = format!("token:refresh:{}:{}", user_id, platform);
        let mut conn = self.get_connection().await?;
        let result: Option<String> = conn.get(&key).await?;
        Ok(result)
    }

    /// 删除指定平台的访问令牌
    async fn delete_access_token_for_platform(&self, user_id: &str, platform: i32) -> Result<(), Error> {
        let key = format!("token:access:{}:{}", user_id, platform);
        let mut conn = self.get_connection().await?;
        conn.del::<_, ()>(&key).await?;
        Ok(())
    }

    /// 删除指定平台的刷新令牌
    async fn delete_refresh_token_for_platform(&self, user_id: &str, platform: i32) -> Result<(), Error> {
        let key = format!("token:refresh:{}:{}", user_id, platform);
        let mut conn = self.get_connection().await?;
        conn.del::<_, ()>(&key).await?;
        Ok(())
    }

    /// 保存用户注册验证码
    ///
    /// 将验证码与邮箱关联并设置5分钟过期时间
    async fn save_register_code(&self, email: &str, code: &str) -> Result<(), Error> {
        // 设置注册码，有效期5分钟
        let mut conn = self.get_connection().await?;
        // 使用管道执行两个命令
        let mut pipe = redis::pipe();
        pipe.hset(REGISTER_CODE_KEY, email, code)
            .expire(REGISTER_CODE_KEY, REGISTER_CODE_EXPIRE)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// 获取用户注册验证码
    async fn get_register_code(&self, email: &str) -> Result<Option<String>, Error> {
        let mut conn = self.get_connection().await?;
        let result = conn.hget(REGISTER_CODE_KEY, email).await?;
        Ok(result)
    }

    /// 删除用户注册验证码
    ///
    /// 用户注册成功后删除验证码
    async fn del_register_code(&self, email: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        conn.hdel::<_, _, ()>(REGISTER_CODE_KEY, email).await?;
        Ok(())
    }
}
