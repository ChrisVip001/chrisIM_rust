use anyhow::Result;
use chrono::{Utc, TimeZone};
use sqlx::PgPool;
use uuid::Uuid;
use common::proto::friend::FriendshipStatus;

use crate::model::friendship::{Friendship, Friend};

pub struct FriendshipRepository {
    pool: PgPool,
}

impl FriendshipRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    // 创建好友请求
    pub async fn create_friend_request(&self, user_id: Uuid, friend_id: Uuid) -> Result<Friendship> {
        let friendship = Friendship::new(user_id, friend_id);
        
        // 将DateTime<Utc>转换为NaiveDateTime
        let created_at_naive = friendship.created_at.naive_utc();
        let updated_at_naive = friendship.updated_at.naive_utc();
        
        let result = sqlx::query!(
            r#"
            INSERT INTO friendships (id, user_id, friend_id, status, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, user_id, friend_id, status, created_at, updated_at
            "#,
            friendship.id.to_string(),
            friendship.user_id.to_string(),
            friendship.friend_id.to_string(),
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
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
    
    // 接受好友请求
    pub async fn accept_friend_request(&self, user_id: Uuid, friend_id: Uuid) -> Result<Friendship> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        let result = sqlx::query!(
            r#"
            UPDATE friendships
            SET status = $1, updated_at = $2
            WHERE user_id = $3 AND friend_id = $4
            RETURNING id, user_id, friend_id, status, created_at, updated_at
            "#,
            (FriendshipStatus::Accepted as i32).to_string(),
            now_naive,
            friend_id.to_string(),
            user_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Friendship {
            id: Uuid::parse_str(&result.id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            friend_id: Uuid::parse_str(&result.friend_id).unwrap(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
    
    // 拒绝好友请求
    pub async fn reject_friend_request(&self, user_id: Uuid, friend_id: Uuid) -> Result<Friendship> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        let result = sqlx::query!(
            r#"
            UPDATE friendships
            SET status = $1, updated_at = $2
            WHERE user_id = $3 AND friend_id = $4
            RETURNING id, user_id, friend_id, status, created_at, updated_at
            "#,
            (FriendshipStatus::Rejected as i32).to_string(),
            now_naive,
            friend_id.to_string(),
            user_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Friendship {
            id: Uuid::parse_str(&result.id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            friend_id: Uuid::parse_str(&result.friend_id).unwrap(),
            status: result.status.parse::<i32>().unwrap_or(0),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
    
    // 获取好友列表
    pub async fn get_friend_list(&self, user_id: Uuid) -> Result<Vec<Friend>> {
        let friends = sqlx::query!(
            r#"
            SELECT 
                u.id, 
                u.username, 
                u.nickname, 
                u.avatar_url, 
                f.created_at as friendship_created_at
            FROM users u
            JOIN friendships f ON 
                (f.friend_id = u.id AND f.user_id = $1) OR 
                (f.user_id = u.id AND f.friend_id = $1)
            WHERE f.status = $2
            "#,
            user_id.to_string(),
            (FriendshipStatus::Accepted as i32).to_string()
        )
        .fetch_all(&self.pool)
        .await?;
        
        let result = friends
            .into_iter()
            .map(|f| Friend {
                id: Uuid::parse_str(&f.id).unwrap(),
                username: f.username,
                nickname: f.nickname,
                avatar_url: f.avatar_url,
                friendship_created_at: Utc.from_utc_datetime(&f.friendship_created_at),
            })
            .collect();
        
        Ok(result)
    }
    
    // 获取好友请求列表
    pub async fn get_friend_requests(&self, user_id: Uuid) -> Result<Vec<Friendship>> {
        let requests = sqlx::query!(
            r#"
            SELECT id, user_id, friend_id, status, created_at, updated_at
            FROM friendships
            WHERE friend_id = $1 AND status = $2
            "#,
            user_id.to_string(),
            (FriendshipStatus::Pending as i32).to_string()
        )
        .fetch_all(&self.pool)
        .await?;
        
        let result = requests
            .into_iter()
            .map(|r| Friendship {
                id: Uuid::parse_str(&r.id).unwrap(),
                user_id: Uuid::parse_str(&r.user_id).unwrap(),
                friend_id: Uuid::parse_str(&r.friend_id).unwrap(),
                status: r.status.parse::<i32>().unwrap_or(0),
                created_at: Utc.from_utc_datetime(&r.created_at),
                updated_at: Utc.from_utc_datetime(&r.updated_at),
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
    pub async fn check_friendship(&self, user_id: Uuid, friend_id: Uuid) -> Result<Option<FriendshipStatus>> {
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
}