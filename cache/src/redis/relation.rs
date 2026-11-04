//! 关系域实现：好友关系、黑名单、群成员

use async_trait::async_trait;
use contract::error::Error;

use super::*;
use crate::traits::RelationCache;
use crate::BlacklistCheckResult;
use redis::AsyncCommands;

#[async_trait]
impl RelationCache for RedisCache {
    /// 保存双向好友关系
    async fn save_bidirectional_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 创建两个用户的好友集合键
        let key1 = format!("{}:{}", USER_FRIENDS_SET_PREFIX, user_id);
        let key2 = format!("{}:{}", USER_FRIENDS_SET_PREFIX, friend_id);

        // 使用管道批量执行命令，提高效率
        let mut pipeline = redis::pipe();
        pipeline
            .sadd(&key1, friend_id)
            .sadd(&key2, user_id);

        // 执行管道命令
        pipeline.query_async::<()>(&mut conn).await.map_err(|e| {
            Error::Internal(format!("保存好友关系失败: {}", e))
        })?;

        Ok(())
    }

    /// 检查双向好友关系是否存在
    async fn check_friendship_exists(&self, user_id: &str, friend_id: &str) -> Result<bool, Error> {
        let mut conn = self.get_connection().await?;

        // 构建键
        let key = format!("{}:{}", USER_FRIENDS_SET_PREFIX, user_id);

        // 检查集合中是否包含好友ID
        let exists: bool = conn.sismember(&key, friend_id).await.map_err(|e| {
            Error::Internal(format!("检查好友关系失败: {}", e))
        })?;

        Ok(exists)
    }

    /// 删除双向好友关系
    async fn delete_bidirectional_friendship(&self, user_id: &str, friend_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 创建两个用户的好友集合键
        let key1 = format!("{}:{}", USER_FRIENDS_SET_PREFIX, user_id);
        let key2 = format!("{}:{}", USER_FRIENDS_SET_PREFIX, friend_id);

        // 使用管道批量执行命令，提高效率
        let mut pipeline = redis::pipe();
        pipeline
            .srem(&key1, friend_id)
            .srem(&key2, user_id);

        // 执行管道命令
        pipeline.query_async::<()>(&mut conn).await.map_err(|e| {
            Error::Internal(format!("删除好友关系失败: {}", e))
        })?;

        Ok(())
    }

    /// 获取用户的所有好友ID
    async fn get_all_friend_ids(&self, user_id: &str) -> Result<Vec<String>, Error> {
        let mut conn = self.get_connection().await?;

        // 构建键
        let key = format!("{}:{}", USER_FRIENDS_SET_PREFIX, user_id);

        // 获取集合中的所有成员
        let friends: Vec<String> = conn.smembers(&key).await.map_err(|e| {
            Error::Internal(format!("获取好友列表失败: {}", e))
        })?;

        Ok(friends)
    }

    /// 添加用户黑名单关系
    async fn add_user_to_blacklist(&self, user_id: &str, blocked_user_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 构建键
        let key = format!("{}:{}", USER_BLACKLIST_PREFIX, user_id);

        // 将被拉黑用户ID添加到黑名单集合
        conn.sadd::<_, _, ()>(&key, blocked_user_id).await.map_err(|e| {
            Error::Internal(format!("添加用户到黑名单失败: {}", e))
        })?;

        Ok(())
    }

    /// 从黑名单中移除用户
    async fn remove_user_from_blacklist(&self, user_id: &str, blocked_user_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;

        // 构建键
        let key = format!("{}:{}", USER_BLACKLIST_PREFIX, user_id);

        // 从黑名单集合中移除用户ID
        conn.srem::<_, _, ()>(&key, blocked_user_id).await.map_err(|e| {
            Error::Internal(format!("从黑名单中移除用户失败: {}", e))
        })?;

        Ok(())
    }

    /// 检查用户是否在黑名单中
    async fn is_user_in_blacklist(&self, user_id: &str, target_user_id: &str) -> Result<bool, Error> {
        let mut conn = self.get_connection().await?;

        // 构建键
        let key = format!("{}:{}", USER_BLACKLIST_PREFIX, user_id);

        // 检查目标用户ID是否在黑名单集合中
        let is_blocked: bool = conn.sismember(&key, target_user_id).await.map_err(|e| {
            Error::Internal(format!("检查用户是否在黑名单中失败: {}", e))
        })?;

        Ok(is_blocked)
    }

    /// 获取用户的黑名单列表
    async fn get_user_blacklist(&self, user_id: &str) -> Result<Vec<String>, Error> {
        let mut conn = self.get_connection().await?;

        // 构建键
        let key = format!("{}:{}", USER_BLACKLIST_PREFIX, user_id);

        // 获取黑名单集合中的所有成员
        let blacklist: Vec<String> = conn.smembers(&key).await.map_err(|e| {
            Error::Internal(format!("获取用户黑名单失败: {}", e))
        })?;

        Ok(blacklist)
    }

    /// 双向检查黑名单关系
    async fn check_bidirectional_blacklist(&self, user_id1: &str, user_id2: &str) -> Result<BlacklistCheckResult, Error> {
        // 检查用户1是否将用户2加入黑名单
        let user1_blocked_user2 = self.is_user_in_blacklist(user_id1, user_id2).await?;

        // 检查用户2是否将用户1加入黑名单
        let user2_blocked_user1 = self.is_user_in_blacklist(user_id2, user_id1).await?;

        // 创建并返回黑名单检查结果
        Ok(BlacklistCheckResult::new(user1_blocked_user2, user2_blocked_user1))
    }

    /// 查询群组成员ID列表
    ///
    /// Redis中的群组成员信息以集合形式存储，键为 group_members_id:group_id
    async fn query_group_members_id(&self, group_id: &str) -> Result<Vec<String>, Error> {
        // 生成键
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        // 从Redis查询值
        let mut conn = self.get_connection().await?;

        let result: Vec<String> = conn.smembers(&key).await?;
        Ok(result)
    }

    /// 保存群组成员ID列表
    ///
    /// 批量添加成员到群组，通常在创建群组时调用
    async fn save_group_members_id(
        &self,
        group_id: &str,
        members_id: Vec<String>,
    ) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        // 通过Redis管道为群组添加每个成员
        let mut pipe = redis::pipe();
        for member in members_id {
            pipe.sadd(&key, &member);
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 添加单个成员到群组
    async fn add_group_member_id(&self, member_id: &str, group_id: &str) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.sadd::<_, _, ()>(&key, member_id).await?;
        Ok(())
    }

    /// 从群组移除单个成员
    async fn remove_group_member_id(&self, group_id: &str, member_id: &str) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.srem::<_, _, ()>(&key, member_id).await?;
        Ok(())
    }

    /// 批量从群组移除成员
    async fn remove_group_member_batch(
        &self,
        group_id: &str,
        member_id: &[&str],
    ) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.srem::<_, _, ()>(&key, member_id).await?;
        Ok(())
    }

    /// 删除群组的所有成员
    async fn del_group_members(&self, group_id: &str) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.del::<_, ()>(&key).await?;
        Ok(())
    }
}
