use anyhow::Result;
use chrono::{TimeZone, Utc};
use common::proto::friend::FriendshipStatus;
use sqlx::{PgPool, Row, FromRow, types::chrono::NaiveDateTime};
use uuid::Uuid;

use crate::model::friendship::{Friend, Friendship};

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
        user_id: Uuid,
        friend_id: Uuid,
        message: String,
    ) -> Result<Friendship> {
        let friendship = Friendship::new(user_id, friend_id,message);

        // 将DateTime<Utc>转换为NaiveDateTime
        let created_at_naive = friendship.created_at.naive_utc();
        let updated_at_naive = friendship.updated_at.naive_utc();

        let result = sqlx::query!(
            r#"
            INSERT INTO friendships (id, user_id, friend_id, message,status, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6,$7)
            RETURNING id, user_id, friend_id, message,status, created_at, updated_at
            "#,
            friendship.id.to_string(),
            friendship.user_id.to_string(),
            friendship.friend_id.to_string(),
            friendship.message.to_string(),
            friendship.status.to_string(),
            created_at_naive,
            updated_at_naive
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Friendship {
            id: Uuid::parse_str(&result.id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            friend_id: Uuid::parse_str(&result.friend_id).unwrap(),
            message: result.message.unwrap_or_default(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
            reject_reason: None,
        })
    }

    // 接受好友请求
    pub async fn accept_friend_request(
        &self,
        user_id: Uuid,
        friend_id: Uuid,
    ) -> Result<Friendship> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 开始事务
        let mut tx = self.pool.begin().await?;

        // 1. 更新friendships表中的状态为已接受
        let result = sqlx::query!(
            r#"
            UPDATE friendships
            SET status = $1, updated_at = $2
            WHERE user_id = $3 AND friend_id = $4
            RETURNING id, user_id, friend_id, message,status, created_at, updated_at
            "#,
            (FriendshipStatus::Accepted as i32).to_string(),
            now_naive,
            user_id.to_string(),
            friend_id.to_string()
        )
        .fetch_one(&mut *tx)
        .await?;

        // 2. 为用户和好友双向插入好友关系
        // 用户 -> 好友方向
        let relation_id1 = Uuid::new_v4();
        sqlx::query!(
            r#"
            INSERT INTO friend_relation (id, user_id, friend_id, status, create_time)
            VALUES ($1, $2, $3, 1, $4)
            ON CONFLICT (user_id, friend_id) DO NOTHING
            "#,
            relation_id1.to_string(),
            user_id.to_string(),
            friend_id.to_string(),
            now_naive
        )
        .execute(&mut *tx)
        .await?;

        // 好友 -> 用户方向
        let relation_id2 = Uuid::new_v4();
        sqlx::query!(
            r#"
            INSERT INTO friend_relation (id, user_id, friend_id, status, create_time)
            VALUES ($1, $2, $3, 1, $4)
            ON CONFLICT (user_id, friend_id) DO NOTHING
            "#,
            relation_id2.to_string(),
            friend_id.to_string(),
            user_id.to_string(),
            now_naive
        )
        .execute(&mut *tx)
        .await?;

        // 提交事务
        tx.commit().await?;

        Ok(Friendship {
            id: Uuid::parse_str(&result.id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            friend_id: Uuid::parse_str(&result.friend_id).unwrap(),
            message: result.message.unwrap_or_default(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
            reject_reason: None,
        })
    }

    // 拒绝好友请求
    pub async fn reject_friend_request(
        &self,
        user_id: Uuid,
        friend_id: Uuid,
        reason: Option<String>,
    ) -> Result<Friendship> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        let result = sqlx::query!(
            r#"
            UPDATE friendships
            SET status = $1, updated_at = $2, reject_reason = $3
            WHERE user_id = $4 AND friend_id = $5
            RETURNING id, user_id, friend_id, message, status, created_at, updated_at, reject_reason
            "#,
            (FriendshipStatus::Rejected as i32).to_string(),
            now_naive,
            reason.as_deref(),
            user_id.to_string(),
            friend_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Friendship {
            id: Uuid::parse_str(&result.id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            friend_id: Uuid::parse_str(&result.friend_id).unwrap(),
            message: result.message.unwrap_or_default(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
            reject_reason: result.reject_reason,
        })
    }

    // 获取好友列表
    pub async fn get_friend_list(
        &self,
        user_id: Uuid,
        page: Option<i64>,
        page_size: Option<i64>,
        sort_by: Option<String>,
    ) -> Result<Vec<Friend>> {
        // 默认分页参数
        let page = page.unwrap_or(1);
        let page_size = page_size.unwrap_or(20);
        let offset = (page - 1) * page_size;
        
        // 排序字段处理 - 使用安全的预定义字段排序
        let order_by = match sort_by.as_deref() {
            Some("username_asc") => "u.username ASC",
            Some("username_desc") => "u.username DESC",
            Some("created_at_asc") => "fr.create_time ASC",
            Some("created_at_desc") => "fr.create_time DESC",
            _ => "fr.create_time DESC", // 默认按创建时间降序
        };

        // 构建SQL查询字符串
        let query = format!(
            r#"
            SELECT 
                u.id::text, 
                u.username, 
                u.nickname, 
                u.avatar_url, 
                fr.create_time as friendship_created_at, 
                fr.remark
            FROM users u
            JOIN friend_relation fr ON fr.friend_id = u.id 
            WHERE fr.user_id = $1 AND fr.status = 1
            ORDER BY {}
            LIMIT $2 OFFSET $3
            "#,
            order_by
        );
        
        // 创建一个中间结构体用于接收数据库结果
        #[derive(sqlx::FromRow)]
        struct FriendRow {
            id: String,
            username: String,
            nickname: Option<String>,
            avatar_url: Option<String>,
            friendship_created_at: NaiveDateTime,
            remark: Option<String>,
        }
        
        // 使用query_as执行查询并映射结果
        let rows = sqlx::query_as::<_, FriendRow>(&query)
            .bind(user_id.to_string())
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        
        // 将FriendRow转换为Friend
        let friends = rows
            .into_iter()
            .map(|row| Friend {
                id: Uuid::parse_str(&row.id).unwrap(),
                username: row.username,
                nickname: row.nickname,
                avatar_url: row.avatar_url,
                friendship_created_at: Utc.from_utc_datetime(&row.friendship_created_at),
                remark: row.remark,
            })
            .collect();
            
        Ok(friends)
    }

    // 获取好友请求列表
    pub async fn get_friend_requests(&self, user_id: Uuid) -> Result<Vec<Friendship>> {
        let requests = sqlx::query!(
            r#"
            SELECT id, user_id, friend_id, message, status, created_at, updated_at,reject_reason
            FROM friendships
            WHERE friend_id = $1 or user_id=$2
            "#,
            user_id.to_string(),
            user_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let result = requests
            .into_iter()
            .map(|r| Friendship {
                id: Uuid::parse_str(&r.id).unwrap(),
                user_id: Uuid::parse_str(&r.user_id).unwrap(),
                friend_id: Uuid::parse_str(&r.friend_id).unwrap(),
                message: r.message.unwrap_or_default(),
                status: r.status.parse::<i32>().unwrap_or(0),
                created_at: Utc.from_utc_datetime(&r.created_at),
                updated_at: Utc.from_utc_datetime(&r.updated_at),
                reject_reason: Some(r.reject_reason.unwrap_or_default()),
            })
            .collect();

        Ok(result)
    }

    // 删除好友
    pub async fn delete_friend(&self, user_id: Uuid, friend_id: Uuid) -> Result<bool> {
        let rows_affected = sqlx::query!(
            r#"
            DELETE FROM friendships
            WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)
            "#,
            user_id.to_string(),
            friend_id.to_string()
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }

    // 检查好友关系
    pub async fn check_friendship(
        &self,
        user_id: Uuid,
        friend_id: Uuid,
    ) -> Result<Option<FriendshipStatus>> {
        let result = sqlx::query!(
            r#"
            SELECT status
            FROM friendships
            WHERE (user_id = $1 AND friend_id = $2) OR (user_id = $2 AND friend_id = $1)
            "#,
            user_id.to_string(),
            friend_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|r| {
            let status_code = r.status.parse::<i32>().unwrap_or(0);
            match status_code {
                0 => FriendshipStatus::Pending,
                1 => FriendshipStatus::Accepted,
                2 => FriendshipStatus::Rejected,
                3 => FriendshipStatus::Blocked,
                _ => FriendshipStatus::Pending,
            }
        }))
    }

    // 检查用户是否存在
    pub async fn check_user_exists(&self, user_id: Uuid) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM users
                WHERE id = $1
            ) AS "exists!"
            "#,
            user_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(result.exists)
    }
}