use anyhow::Result;
use chrono::{TimeZone, Utc};
use common::proto::friend::FriendshipStatus;
use sqlx::{PgPool, Row, FromRow, types::chrono::NaiveDateTime};
use uuid::Uuid;

use crate::model::friendship::{Friend, Friendship, FriendGroup, PotentialFriend, DetailedFriend};
use crate::model::user_blacklist::UserBlacklist;

pub struct FriendshipRepository {
    pool: PgPool,
}

impl FriendshipRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 创建好友请求
    pub async fn create_friend_request(
        &self,
        user_id: &str,
        friend_id: &str,
        message: String,
    ) -> Result<Friendship> {
        let friendship = Friendship::new(user_id.to_string(), friend_id.to_string(), message);

        // // 将DateTime<Utc>转换为NaiveDateTime
        let created_at_naive = friendship.created_at.naive_utc();
        let updated_at_naive = friendship.updated_at.naive_utc();

        let result = sqlx::query!(
            r#"
            INSERT INTO friendships (id, user_id, friend_id, message, status, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, user_id, friend_id, message, status, created_at, updated_at
            "#,
            friendship.id,
            friendship.user_id,
            friendship.friend_id,
            friendship.message,
            friendship.status.to_string(),
            created_at_naive,
            updated_at_naive
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Friendship {
            id: result.id,
            user_id: result.user_id,
            friend_id: result.friend_id,
            message: result.message.unwrap_or_default(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
            reject_reason: None,
            friend_username: None,
            friend_nickname: None,
            friend_avatar_url: None,
        })
    }

    // 接受好友请求
    pub async fn accept_friend_request(
        &self,
        user_id: &str,
        request_id: &str,
    ) -> Result<Friendship> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 开始事务
        let mut tx = self.pool.begin().await?;

        // 1. 获取好友请求信息，确保请求ID与用户ID匹配
        let friendship_result = sqlx::query!(
            r#"
            SELECT id, user_id, friend_id FROM friendships
            WHERE id = $1 AND friend_id = $2 AND status = '0'
            "#,
            request_id,
            user_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        let friendship = match friendship_result {
            Some(fs) => fs,
            None => return Err(anyhow::anyhow!("好友请求不存在或已处理")),
        };

        // 2. 更新friendships表中的状态为已接受
        let result = sqlx::query!(
            r#"
            UPDATE friendships
            SET status = $1, updated_at = $2
            WHERE id = $3
            RETURNING id, user_id, friend_id, message, status, created_at, updated_at
            "#,
            (FriendshipStatus::Accepted as i32).to_string(),
            now_naive,
            request_id
        )
        .fetch_one(&mut *tx)
        .await?;

        // 3. 为用户和好友双向插入好友关系
        // 用户 -> 好友方向
        let relation_id1 = Uuid::new_v4().to_string();
        sqlx::query!(
            r#"
            INSERT INTO friend_relation (id, user_id, friend_id, status, created_at)
            VALUES ($1, $2, $3, 1, $4)
            ON CONFLICT (user_id, friend_id) DO NOTHING
            "#,
            relation_id1,
            user_id,
            friendship.user_id,
            now_naive
        )
        .execute(&mut *tx)
        .await?;

        // 好友 -> 用户方向
        let relation_id2 = Uuid::new_v4().to_string();
        sqlx::query!(
            r#"
            INSERT INTO friend_relation (id, user_id, friend_id, status, created_at)
            VALUES ($1, $2, $3, 1, $4)
            ON CONFLICT (user_id, friend_id) DO NOTHING
            "#,
            relation_id2,
            friendship.user_id,
            user_id,
            now_naive
        )
        .execute(&mut *tx)
        .await?;

        // 提交事务
        tx.commit().await?;

        Ok(Friendship {
            id: result.id,
            user_id: result.user_id,
            friend_id: result.friend_id,
            message: result.message.unwrap_or_default(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
            reject_reason: None,
            friend_username: None,
            friend_nickname: None,
            friend_avatar_url: None,
        })
    }

    // 拒绝好友请求
    pub async fn reject_friend_request(
        &self,
        user_id: &str,
        reason: Option<String>,
        request_id: &str,
    ) -> Result<Friendship> {
        // 1. 获取好友请求信息，确保请求ID与用户ID匹配
        let friendship_result = sqlx::query!(
            r#"
            SELECT id, user_id, friend_id FROM friendships
            WHERE id = $1 AND friend_id = $2 AND status = '0'
            "#,
            request_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;

        let _friendship = match friendship_result {
            Some(fs) => fs,
            None => return Err(anyhow::anyhow!("好友请求不存在或已处理")),
        };

        let now = Utc::now();
        let now_naive = now.naive_utc();
        let result = sqlx::query!(
            r#"
            UPDATE friendships
            SET status = $1, updated_at = $2, reject_reason = $3
            WHERE id = $4
            RETURNING id, user_id, friend_id, message, status, created_at, updated_at, reject_reason
            "#,
            (FriendshipStatus::Rejected as i32).to_string(),
            now_naive,
            reason.as_deref(),
            request_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Friendship {
            id: result.id,
            user_id: result.user_id,
            friend_id: result.friend_id,
            message: result.message.unwrap_or_default(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
            reject_reason: result.reject_reason,
            friend_username: None,
            friend_nickname: None,
            friend_avatar_url: None,
        })
    }

    // 获取好友列表
    pub async fn get_friend_list(
        &self,
        user_id: &str,
        page: Option<i64>,
        page_size: Option<i64>,
        sort_by: Option<String>,
        keyword: Option<String>,
    ) -> Result<Vec<Friend>> {
        // 默认分页参数
        let page = page.unwrap_or(1);
        let page_size = page_size.unwrap_or(20);
        let offset = (page - 1) * page_size;
        
        // 排序字段处理 - 使用安全的预定义字段排序
        let order_by = match sort_by.as_deref() {
            Some("username_asc") => "u.username ASC",
            Some("username_desc") => "u.username DESC",
            Some("created_at_asc") => "fr.created_at ASC",
            Some("created_at_desc") => "fr.created_at DESC",
            _ => "fr.created_at DESC", // 默认按创建时间降序
        };

        // 构建基础SQL查询
        let mut base_query = format!(
            r#"
            SELECT 
                u.id::text, 
                u.username, 
                u.nickname, 
                u.avatar_url, 
                fr.created_at as friendship_created_at, 
                fr.remark
            FROM users u
            JOIN friend_relation fr ON fr.friend_id = u.id 
            WHERE fr.user_id = $1 AND fr.status = 1
            "#
        );
        
        // 创建一个中间结构体用于接收数据库结果
        #[derive(sqlx::FromRow)]
        struct FriendRow {
            id: String,
            username: Option<String>,
            nickname: Option<String>,
            avatar_url: Option<String>,
            friendship_created_at: NaiveDateTime,
            remark: Option<String>,
        }
        
        let rows = if let Some(keyword) = &keyword {
            // 如果有关键词，使用参数化查询
            let search_query = format!(
                r#"
                {}
                AND (
                    u.username ILIKE $4 OR 
                    u.nickname ILIKE $4 OR 
                    fr.remark ILIKE $4
                )
                ORDER BY {}
                LIMIT $2 OFFSET $3
                "#, 
                base_query, order_by
            );
            
            let search_pattern = format!("%{}%", keyword);
            
            sqlx::query_as::<_, FriendRow>(&search_query)
                .bind(user_id)
                .bind(page_size)
                .bind(offset)
                .bind(search_pattern)
                .fetch_all(&self.pool)
                .await?
        } else {
            // 没有关键词，使用基本查询
            let query = format!(
                r#"
                {}
                ORDER BY {}
                LIMIT $2 OFFSET $3
                "#, 
                base_query, order_by
            );
            
            sqlx::query_as::<_, FriendRow>(&query)
                .bind(user_id)
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        };
        
        // 将FriendRow转换为Friend
        let friends = rows
            .into_iter()
            .map(|row| Friend {
                id: row.id,
                username: row.username,
                nickname: row.nickname,
                avatar_url: row.avatar_url,
                friendship_created_at: Utc.from_utc_datetime(&row.friendship_created_at),
                remark: row.remark,
            })
            .collect();
            
        Ok(friends)
    }

    // 获取好友请求总数
    pub async fn count_friend_requests(&self, user_id: &str) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            SELECT COUNT(*) as "count!" 
            FROM friendships 
            WHERE friend_id = $1 OR user_id = $2
            "#,
            user_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result.count)
    }

    /// 获取好友请求列表
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `page` - 页码，默认为1
    /// * `page_size` - 每页数量，默认为20
    /// 
    /// # 返回
    /// * `Result<Vec<Friendship>>` - 好友请求列表
    /// 
    /// # 说明
    /// 1. 获取指定用户的好友请求列表，包括发送和接收的请求
    /// 2. 对于状态为 Pending 且创建时间超过3天的请求，状态会被标记为 Expired
    /// 3. 结果按创建时间降序排序
    pub async fn get_friend_requests(
        &self,
        user_id: &str,
        page: Option<i64>,
        page_size: Option<i64>,
    ) -> Result<Vec<Friendship>> {
        // 设置分页参数
        let page = page.unwrap_or(1);
        let page_size = page_size.unwrap_or(20);
        let offset = (page - 1) * page_size;

        // 查询好友请求列表
        let requests = sqlx::query!(
            r#"
            SELECT 
                f.id, 
                f.user_id, 
                f.friend_id, 
                f.message, 
                f.status, 
                f.created_at, 
                f.updated_at, 
                f.reject_reason,
                u.username as friend_username,
                u.nickname as friend_nickname,
                u.avatar_url as friend_avatar_url
            FROM friendships f
            LEFT JOIN users u ON (
                CASE 
                    WHEN f.user_id = $1 THEN f.friend_id = u.id
                    ELSE f.user_id = u.id
                END
            )
            WHERE f.friend_id = $1 OR f.user_id = $1
            ORDER BY f.created_at DESC
            LIMIT $2 OFFSET $3
            "#,
            user_id,
            page_size,
            offset
        )
        .fetch_all(&self.pool)
        .await?;

        // 计算过期时间点（当前时间减去3天）
        let now = Utc::now();
        let three_days_ago = now - chrono::Duration::days(3);

        // 处理查询结果
        let result = requests
            .into_iter()
            .map(|r| {
                // 解析状态值
                let mut status = r.status.parse::<i32>().unwrap_or(0);

                // 判断请求是否过期：
                // 1. 状态必须为 Pending (0)
                // 2. 创建时间必须超过3天
                if status == 0 && Utc.from_utc_datetime(&r.created_at) < three_days_ago {
                    status = 4; // 设置为 Expired 状态
                }

                // 构建 Friendship 对象
                Friendship {
                    id: r.id,
                    user_id: r.user_id,
                    friend_id: r.friend_id,
                    message: r.message.unwrap_or_default(),
                    status,
                    created_at: Utc.from_utc_datetime(&r.created_at),
                    updated_at: Utc.from_utc_datetime(&r.updated_at),
                    reject_reason: r.reject_reason,
                    friend_username: r.friend_username,
                    friend_nickname: r.friend_nickname,
                    friend_avatar_url: r.friend_avatar_url,
                }
            })
            .collect();

        Ok(result)
    }

    // 删除好友
    pub async fn delete_friend(&self, user_id: &str, friend_id: &str) -> Result<bool> {
        // 开始事务
        let mut tx = self.pool.begin().await?;

        // 1. 删除好友关系
        let delete_relation = sqlx::query!(
            r#"
            DELETE FROM friend_relation
            WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)
            "#,
            user_id,
            friend_id
        )
        .execute(&mut *tx)
        .await?;

        // 2. 删除好友申请记录
        let delete_request = sqlx::query!(
            r#"
            DELETE FROM friendships
            WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)
            "#,
            user_id,
            friend_id
        )
        .execute(&mut *tx)
        .await?;

        // 提交事务
        tx.commit().await?;

        Ok(delete_relation.rows_affected() > 0 || delete_request.rows_affected() > 0)
    }

    // 检查好友关系状态
    // 检查好友关系
    pub async fn check_friendship(
        &self,
        user_id: &str,
        friend_id: &str,
    ) -> Result<Option<FriendshipStatus>> {
        // 首先检查是否在黑名单中
        let blacklist_exists = sqlx::query!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_blacklist
                WHERE user_id = $1 AND blocked_user_id = $2
            ) AS "exists!"
            "#,
            user_id,
            friend_id
        )
        .fetch_one(&self.pool)
        .await?;
        
        if blacklist_exists.exists {
            return Ok(Some(FriendshipStatus::Blocked));
        }
        
        // 检查对方是否把自己拉黑
        let reverse_blacklist_exists = sqlx::query!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_blacklist
                WHERE user_id = $1 AND blocked_user_id = $2
            ) AS "exists!"
            "#,
            friend_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        
        if reverse_blacklist_exists.exists {
            return Ok(Some(FriendshipStatus::Blocked));
        }
        
        // 然后检查 friend_relation 表中的状态
        let relation_result = sqlx::query!(
            r#"
            SELECT status FROM friend_relation
            WHERE user_id = $1 AND friend_id = $2
            "#,
            user_id,
            friend_id
        )
        .fetch_optional(&self.pool)
        .await?;

        // 如果在 friend_relation 表中找到记录，直接返回对应状态
        if let Some(relation) = relation_result {
            let status = match relation.status {
                1 => FriendshipStatus::Accepted,
                2 => FriendshipStatus::Blocked,
                _ => FriendshipStatus::Accepted,
            };
            return Ok(Some(status));
        }

        // 如果在 friend_relation 表中没有找到记录，则检查 friendships 表
        let result = sqlx::query!(
            r#"
            SELECT status, created_at
            FROM friendships
            WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)
            "#,
            user_id,
            friend_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|r| {
            let mut status_code = r.status.parse::<i32>().unwrap_or(0);

            // 判断请求是否过期：
            // 1. 状态必须为 Pending (0)
            // 2. 创建时间必须超过3天
            if status_code == 0 {
                let now = Utc::now();
                let three_days_ago = now - chrono::Duration::days(3);
                if Utc.from_utc_datetime(&r.created_at) < three_days_ago {
                    status_code = 4; // 设置为 Expired 状态
                }
            }

            match status_code {
                0 => FriendshipStatus::Pending,
                1 => FriendshipStatus::Accepted,
                2 => FriendshipStatus::Rejected,
                3 => FriendshipStatus::Blocked,
                4 => FriendshipStatus::Expired,
                _ => FriendshipStatus::Pending,
            }
        }))
    }

    /// 搜索潜在好友
    /// 
    /// 根据custom_id或手机号搜索用户，并返回与当前用户的好友关系
    pub async fn search_potential_friends(
        &self,
        user_id: &str,
        search_term: &str,
    ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>, i32)>> {
        // 构建SQL查询，自动匹配custom_id或手机号
        let query = r#"
            SELECT 
                u.id, 
                u.username, 
                u.nickname, 
                u.avatar_url, 
                u.phone,
                COALESCE(fr.status, -1) as friendship_status
            FROM 
                users u
            LEFT JOIN 
                friend_relation fr ON (fr.user_id = $1 AND fr.friend_id = u.id)
            WHERE 
                u.id != $1 AND (u.custom_id = $2 OR u.phone = $2)
            ORDER BY 
                u.id
        "#;
        
        // 执行查询
        let rows = sqlx::query(query)
            .bind(user_id)
            .bind(search_term)
            .fetch_all(&self.pool)
            .await?;
            
        // 提取结果
        let results = rows
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("id"),
                    row.get::<String, _>("username"),
                    row.get::<Option<String>, _>("nickname"),
                    row.get::<Option<String>, _>("avatar_url"),
                    row.get::<Option<String>, _>("phone"),
                    row.get::<i32, _>("friendship_status"),
                )
            })
            .collect();
            
        Ok(results)
    }
    
    // 检查用户是否存在
    pub async fn check_user_exists(&self, user_id: &str) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS (SELECT 1 FROM users WHERE id = $1) as "exists!"
            "#,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result.exists)
    }

    // 获取好友总数
    pub async fn count_friends(&self, user_id: &str, keyword: Option<String>) -> Result<i64> {
        if let Some(keyword) = keyword {
            // 有关键词时使用LIKE查询
            let search_pattern = format!("%{}%", keyword);
            let result = sqlx::query!(
                r#"
                SELECT COUNT(*) as "count!"
                FROM friend_relation fr
                JOIN users u ON fr.friend_id = u.id
                WHERE fr.user_id = $1 AND fr.status = 1
                AND (
                    u.username ILIKE $2 OR 
                    u.nickname ILIKE $2 OR 
                    fr.remark ILIKE $2
                )
                "#,
                user_id,
                search_pattern
            )
            .fetch_one(&self.pool)
            .await?;
            
            Ok(result.count)
        } else {
            // 无关键词时的常规查询
            let result = sqlx::query!(
                r#"
                SELECT COUNT(*) as "count!"
                FROM friend_relation
                WHERE user_id = $1 AND status = 1
                "#,
                user_id
            )
            .fetch_one(&self.pool)
            .await?;
            
            Ok(result.count)
        }
    }

    // 拉黑用户（增强版 - 使用用户黑名单表）
    pub async fn block_user(&self, user_id: &str, blocked_user_id: &str, reason: Option<String>) -> Result<UserBlacklist> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        // 1. 记录到用户黑名单表
        let blacklist = UserBlacklist::new(
            user_id.to_string(), 
            blocked_user_id.to_string(), 
            reason.clone()
        );
        
        let created_at_naive = blacklist.created_at.naive_utc();
        
        sqlx::query!(
            r#"
            INSERT INTO user_blacklist (id, user_id, blocked_user_id, reason, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, blocked_user_id) 
            DO UPDATE SET reason = $4, created_at = $5
            "#,
            blacklist.id,
            blacklist.user_id,
            blacklist.blocked_user_id,
            blacklist.reason,
            created_at_naive
        )
        .execute(&mut *tx)
        .await?;
        
        // 2. 如果是好友关系，只修改状态为拉黑，不删除关系
        // 查询是否存在好友关系
        let friend_relation = sqlx::query!(
            r#"
            SELECT id, status FROM friend_relation
            WHERE user_id = $1 AND friend_id = $2
            "#,
            user_id,
            blocked_user_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        
        if let Some(relation) = friend_relation {
            if relation.status == 1 {  // 如果是接受状态的好友关系
                // 只更新状态为拉黑状态
                sqlx::query!(
                    r#"
                    UPDATE friend_relation
                    SET status = 2, updated_at = $1
                    WHERE id = $2
                    "#,
                    now_naive,
                    relation.id
                )
                .execute(&mut *tx)
                .await?;
            }
        }
        
        tx.commit().await?;
            
        Ok(blacklist)
    }

    // 解除拉黑（增强版 - 使用用户黑名单表）
    pub async fn unblock_user(&self, user_id: &str, blocked_user_id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        // 1. 从用户黑名单表中删除记录
        let blacklist_result = sqlx::query!(
            r#"
            DELETE FROM user_blacklist
            WHERE user_id = $1 AND blocked_user_id = $2
            RETURNING id
            "#,
            user_id,
            blocked_user_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        
        let blacklist_removed = blacklist_result.is_some();
        
        // 2. 查询是否存在状态为拉黑的好友关系记录
        let relation = sqlx::query!(
            r#"
            SELECT id, status FROM friend_relation
            WHERE user_id = $1 AND friend_id = $2
            "#,
            user_id,
            blocked_user_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        
        // 3. 如果存在拉黑状态的好友关系记录，将其恢复为接受状态
        if let Some(rel) = relation {
            if rel.status == 2 {  // 状态为拉黑
                sqlx::query!(
                    r#"
                    UPDATE friend_relation
                    SET status = 1, updated_at = $1
                    WHERE id = $2
                    "#,
                    now_naive,
                    rel.id
                )
                .execute(&mut *tx)
                .await?;
            }
        }
        
        tx.commit().await?;
            
        Ok(blacklist_removed)
    }

    // 检查用户是否被拉黑（增强版 - 使用用户黑名单表）
    pub async fn is_user_blocked(&self, user_id: &str, blocked_user_id: &str) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_blacklist
                WHERE user_id = $1 AND blocked_user_id = $2
            ) AS "exists!"
            "#,
            user_id,
            blocked_user_id
        )
        .fetch_one(&self.pool)
        .await?;
            
        Ok(result.exists)
    }
    
    // 获取用户黑名单列表
    pub async fn get_user_blacklist(
        &self,
        user_id: &str,
        page: Option<i64>,
        page_size: Option<i64>,
    ) -> Result<Vec<UserBlacklist>> {
        // 默认分页参数
        let page = page.unwrap_or(1);
        let page_size = page_size.unwrap_or(20);
        let offset = (page - 1) * page_size;
        
        // 查询用户黑名单
        let rows = sqlx::query!(
            r#"
            SELECT 
                ub.id, 
                ub.user_id, 
                ub.blocked_user_id, 
                ub.reason, 
                ub.created_at
            FROM user_blacklist ub
            WHERE ub.user_id = $1
            ORDER BY ub.created_at DESC
            LIMIT $2 OFFSET $3
            "#,
            user_id,
            page_size,
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        
        let blacklist = rows
            .into_iter()
            .map(|row| UserBlacklist {
                id: row.id,
                user_id: row.user_id,
                blocked_user_id: row.blocked_user_id,
                reason: row.reason,
                created_at: Utc.from_utc_datetime(&row.created_at),
            })
            .collect();
        
        Ok(blacklist)
    }
    
    // 获取带用户信息的黑名单列表
    pub async fn get_user_blacklist_with_info(
        &self,
        user_id: &str,
        page: Option<i64>,
        page_size: Option<i64>,
    ) -> Result<Vec<(UserBlacklist, Option<String>, Option<String>, Option<String>)>> {
        // 查询用户黑名单，并联合用户表获取用户信息
        let rows = sqlx::query!(
            r#"
            SELECT 
                ub.id, 
                ub.user_id, 
                ub.blocked_user_id, 
                ub.reason, 
                ub.created_at,
                u.username,
                u.nickname,
                u.avatar_url
            FROM user_blacklist ub
            LEFT JOIN users u ON ub.blocked_user_id = u.id
            WHERE ub.user_id = $1
            ORDER BY ub.created_at DESC
            "#,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;
        
        let blacklist_with_info = rows
            .into_iter()
            .map(|row| {
                let blacklist = UserBlacklist {
                    id: row.id,
                    user_id: row.user_id,
                    blocked_user_id: row.blocked_user_id,
                    reason: row.reason,
                    created_at: Utc.from_utc_datetime(&row.created_at),
                };
                
                (blacklist, row.username, row.nickname, row.avatar_url)
            })
            .collect();
        
        Ok(blacklist_with_info)
    }
    
    // 计算用户黑名单总数
    pub async fn count_user_blacklist(&self, user_id: &str) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            SELECT COUNT(*) as "count!" FROM user_blacklist
            WHERE user_id = $1
            "#,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(result.count)
    }

    // 更新好友分组中的好友
    pub async fn update_group_friends(
        &self,
        group_id: &str,
        user_id: &str,
        friend_ids: &Vec<String>,
    ) -> Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 1. 删除该分组下的所有好友
        sqlx::query!(
            r#"
            DELETE FROM friend_group_relation
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;

        // 2. 逐个添加好友到分组
        let mut added_friend_ids = Vec::new();
        for friend_id in friend_ids {
            // 检查好友关系
            if let Ok(Some(status)) = self.check_friendship(user_id, friend_id).await {
                if status == FriendshipStatus::Accepted {
                    if sqlx::query!(
                        r#"
                        INSERT INTO friend_group_relation (id, user_id, friend_id, group_id, created_at, updated_at)
                        VALUES ($1, $2, $3, $4, $5, $6)
                        ON CONFLICT (user_id, friend_id, group_id) DO UPDATE
                        SET updated_at = $6
                        "#,
                        Uuid::new_v4().to_string(),
                        user_id,
                        friend_id,
                        group_id,
                        now_naive,
                        now_naive
                    )
                    .execute(&mut *tx)
                    .await
                    .is_ok()
                    {
                        added_friend_ids.push(friend_id.clone());
                    }
                }
            }
        }

        tx.commit().await?;
        Ok(added_friend_ids)
    }

    // 创建好友分组
    pub async fn create_friend_group(
        &self,
        user_id: &str,
        group_name: String,
        sort_order: i32,
    ) -> Result<FriendGroup> {
        let group = FriendGroup::new(user_id.to_string(), group_name.clone(), sort_order);

        let created_at_naive = group.created_at.naive_utc();
        let updated_at_naive = group.updated_at.naive_utc();

        let result = sqlx::query!(
            r#"
            INSERT INTO friend_group (id, user_id, group_name, sort_order, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, user_id, group_name, sort_order, created_at, updated_at
            "#,
            group.id,
            group.user_id,
            group.group_name,
            group.sort_order,
            created_at_naive,
            updated_at_naive
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(FriendGroup {
            id: result.id,
            user_id: result.user_id,
            group_name: result.group_name,
            sort_order: result.sort_order.unwrap(),
            created_at: Utc.from_utc_datetime(&result.created_at.unwrap_or_default()),
            updated_at: Utc.from_utc_datetime(&result.updated_at.unwrap_or_default()),
            friend_count: 0,
        })
    }

    // 更新好友分组
    pub async fn update_friend_group(
        &self,
        id: &str,
        user_id: &str,
        group_name: String,
        sort_order: i32,
    ) -> Result<FriendGroup> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        let result = sqlx::query!(
            r#"
            UPDATE friend_group
            SET group_name = $1, sort_order = $2, updated_at = $3
            WHERE id = $4 AND user_id = $5
            RETURNING id, user_id, group_name, sort_order, created_at, updated_at
            "#,
            group_name,
            sort_order,
            now_naive,
            id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(FriendGroup {
            id: result.id,
            user_id: result.user_id,
            group_name: result.group_name,
            sort_order: result.sort_order.unwrap(),
            created_at: Utc.from_utc_datetime(&result.created_at.unwrap_or_default()),
            updated_at: Utc.from_utc_datetime(&result.updated_at.unwrap_or_default()),
            friend_count: 0,
        })
    }

    // 删除好友分组
    pub async fn delete_friend_group(&self, id: &str, user_id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;

        // 1. 删除分组关联的好友
        sqlx::query!(
            r#"
            DELETE FROM friend_group_relation
            WHERE group_id = $1 AND user_id = $2
            "#,
            id,
            user_id
        )
        .execute(&mut *tx)
        .await?;

        // 2. 删除分组
        let result = sqlx::query!(
            r#"
            DELETE FROM friend_group
            WHERE id = $1 AND user_id = $2
            "#,
            id,
            user_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(result.rows_affected() > 0)
    }

    // 获取好友分组列表
    pub async fn get_friend_groups(&self, user_id: &str) -> Result<Vec<FriendGroup>> {
        let groups = sqlx::query!(
            r#"
            SELECT g.*, COUNT(r.friend_id) as "friend_count!: i32"
            FROM friend_group g
            LEFT JOIN friend_group_relation r ON g.id = r.group_id
            WHERE g.user_id = $1
            GROUP BY g.id
            ORDER BY g.sort_order ASC
            "#,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;

        let result = groups
            .into_iter()
            .map(|g| FriendGroup {
                id: g.id,
                user_id: g.user_id,
                group_name: g.group_name,
                sort_order: g.sort_order.unwrap(),
                created_at: Utc.from_utc_datetime(&g.created_at.unwrap_or_default()),
                updated_at: Utc.from_utc_datetime(&g.updated_at.unwrap_or_default()),
                friend_count: g.friend_count,
            })
            .collect();

        Ok(result)
    }

    // 获取分组好友列表
    pub async fn get_group_friends(&self, group_id: &str, user_id: &str) -> Result<Vec<Friend>> {
        let friends = sqlx::query!(
            r#"
            SELECT u.id as friend_id, u.username, u.nickname, u.avatar_url, 
                   fr.created_at as friendship_created_at, NULL as remark
            FROM friend_group_relation gr
            JOIN users u ON gr.friend_id = u.id
            JOIN friend_relation fr ON (fr.user_id = gr.user_id AND fr.friend_id = gr.friend_id)
            WHERE gr.group_id = $1 AND gr.user_id = $2
            ORDER BY fr.created_at DESC
            "#,
            group_id,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;

        let result = friends
            .into_iter()
            .map(|f| Friend {
                id: f.friend_id,
                username: f.username,
                nickname: f.nickname,
                avatar_url: f.avatar_url,
                friendship_created_at: Utc.from_utc_datetime(&f.friendship_created_at),
                remark: f.remark,
            })
            .collect();

        Ok(result)
    }
    
    // 检查分组名称是否重复
    pub async fn check_group_name_exists(&self, user_id: &str, group_name: &str, exclude_group_id: Option<String>) -> Result<bool> {
        let exists = if let Some(id) = exclude_group_id {
            // 更新分组时，排除当前分组
            let result = sqlx::query!(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM friend_group 
                    WHERE user_id = $1 
                    AND group_name = $2
                    AND id != $3
                ) AS "exists!"
                "#,
                user_id,
                group_name,
                id
            )
            .fetch_one(&self.pool)
            .await?;
            result.exists
        } else {
            // 创建分组时，检查是否有重名
            let result = sqlx::query!(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM friend_group 
                    WHERE user_id = $1 
                    AND group_name = $2
                ) AS "exists!"
                "#,
                user_id,
                group_name
            )
            .fetch_one(&self.pool)
            .await?;
            result.exists
        };

        Ok(exists)
    }

    /// 获取好友详细列表（无分页）
    pub async fn get_all_friend_detail_list(
        &self,
        user_id: &str,
    ) -> Result<Vec<DetailedFriend>> {
        // 直接查询所有好友关系
        let query = r#"
            SELECT 
                u.id::text, 
                u.username, 
                u.nickname, 
                u.avatar_url, 
                fr.created_at as friendship_created_at, 
                fr.remark,
                fr.status as relation_status,
                fr.friend_type,
                COALESCE(fr.is_starred, 0) as is_starred,
                COALESCE(fr.is_top, 0) as is_top
            FROM users u
            JOIN friend_relation fr ON (fr.user_id = $1 AND fr.friend_id = u.id)
            WHERE fr.status = 1
            ORDER BY fr.is_top DESC, fr.is_starred DESC, fr.created_at DESC
            "#;
        
        // 创建一个用于接收数据库结果的结构体
        #[derive(sqlx::FromRow)]
        struct FriendDetailRow {
            id: String,
            username: Option<String>,
            nickname: Option<String>,
            avatar_url: Option<String>,
            friendship_created_at: NaiveDateTime,
            remark: Option<String>,
            relation_status: i16,
            friend_type: i16,
            is_starred: i32,
            is_top: i32,
        }
        
        // 执行查询
        let rows = sqlx::query_as::<_, FriendDetailRow>(query)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        
        // 转换结果
        let detailed_friends = rows
            .into_iter()
            .map(|row| DetailedFriend {
                id: row.id,
                username: row.username,
                nickname: row.nickname,
                avatar_url: row.avatar_url,
                friendship_created_at: Utc.from_utc_datetime(&row.friendship_created_at),
                remark: row.remark,
                is_online: false, // 默认离线状态，实际应从在线状态服务获取
                is_starred: row.is_starred == 1,
                is_top: row.is_top == 1,
                relation_status: row.relation_status as i32,
                friend_type: row.friend_type as i32, // 默认为普通好友
            })
            .collect();
        
        Ok(detailed_friends)
    }

    /// 更新好友星标状态
    pub async fn update_friend_star(&self, user_id: &str, friend_id: &str, is_starred: bool) -> Result<bool> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        // 将布尔值转换为整数 (0/1)
        let is_starred_int = if is_starred { 1 } else { 0 };

        let result = sqlx::query!(
            r#"
            UPDATE friend_relation
            SET is_starred = $1, updated_at = $2
            WHERE user_id = $3 AND friend_id = $4 AND status = 1
            "#,
            is_starred_int,
            now_naive,
            user_id,
            friend_id
        )
        .execute(&self.pool)
        .await?;
        
        Ok(result.rows_affected() > 0)
    }

    /// 更新好友置顶状态
    pub async fn update_friend_top(&self, user_id: &str, friend_id: &str, is_top: bool) -> Result<bool> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        // 将布尔值转换为整数 (0/1)
        let is_top_int = if is_top { 1 } else { 0 };

        let result = sqlx::query!(
            r#"
            UPDATE friend_relation
            SET is_top = $1, updated_at = $2
            WHERE user_id = $3 AND friend_id = $4 AND status = 1
            "#,
            is_top_int,
            now_naive,
            user_id,
            friend_id
        )
        .execute(&self.pool)
        .await?;
        
        Ok(result.rows_affected() > 0)
    }

    /// 更新好友备注
    pub async fn update_friend_remark(&self, user_id: &str, friend_id: &str, remark: &str) -> Result<DetailedFriend> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        // 先更新好友备注
        let result = sqlx::query!(
            r#"
            UPDATE friend_relation
            SET remark = $1, updated_at = $2
            WHERE user_id = $3 AND friend_id = $4 AND status = 1
            RETURNING 1 as updated
            "#,
            remark,
            now_naive,
            user_id,
            friend_id
        )
        .fetch_optional(&self.pool)
        .await?;
        
        if result.is_none() {
            return Err(anyhow::anyhow!("更新好友备注失败，可能不是好友关系"));
        }
        
        // 查询更新后的好友详细信息
        let row = sqlx::query!(
            r#"
            SELECT u.id, u.username, u.nickname, u.avatar_url,
                   fr.created_at as friendship_created_at,
                   fr.remark, fr.status as relation_status,
                   fr.is_starred, fr.is_top,
                   fr.friend_type 
            FROM users u
            JOIN friend_relation fr ON fr.friend_id = u.id
            WHERE fr.user_id = $1 AND fr.friend_id = $2 AND fr.status = 1
            "#,
            user_id,
            friend_id
        )
        .fetch_one(&self.pool)
        .await?;
        
        let friend = DetailedFriend {
            id: row.id,
            username: row.username,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            friendship_created_at: Utc.from_utc_datetime(&row.friendship_created_at),
            remark: row.remark,
            is_online: false, // 默认离线状态，实际应从在线状态服务获取
            is_starred: row.is_starred == 1,
            is_top: row.is_top == 1,
            relation_status: row.relation_status as i32,
            friend_type: row.friend_type as i32,
        };
        
        Ok(friend)
    }
}