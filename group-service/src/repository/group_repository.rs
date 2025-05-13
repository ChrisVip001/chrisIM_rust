use anyhow::Result;
use chrono::{Utc, TimeZone};
use sqlx::PgPool;
use uuid::Uuid;

use crate::model::group::{Group, UserGroup};

pub struct GroupRepository {
    pool: PgPool,
}

impl GroupRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    // 创建群组
    pub async fn create_group(&self, name: String, description: String, avatar_url: String, owner_id: Uuid) -> Result<Group> {
        let group = Group::new(name, description, avatar_url, owner_id);
        
        // 将DateTime<Utc>转换为NaiveDateTime
        let created_at_naive = group.created_at.naive_utc();
        let updated_at_naive = group.updated_at.naive_utc();
        
        let result = sqlx::query!(
            r#"
            INSERT INTO groups (id, name, description, avatar_url, owner_id, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, name, description, avatar_url, owner_id, created_at, updated_at
            "#,
            group.id.to_string(),
            group.name,
            group.description,
            group.avatar_url,
            group.owner_id.to_string(),
            created_at_naive,
            updated_at_naive
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Group {
            id: Uuid::parse_str(&result.id).unwrap(),
            name: result.name,
            description: result.description.unwrap_or_default(),
            avatar_url: result.avatar_url.unwrap_or_default(),
            owner_id: Uuid::parse_str(&result.owner_id).unwrap(),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
    
    // 获取群组信息
    pub async fn get_group(&self, group_id: Uuid) -> Result<Group> {
        let result = sqlx::query!(
            r#"
            SELECT id, name, description, avatar_url, owner_id, created_at, updated_at
            FROM groups
            WHERE id = $1
            "#,
            group_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Group {
            id: Uuid::parse_str(&result.id).unwrap(),
            name: result.name,
            description: result.description.unwrap_or_default(),
            avatar_url: result.avatar_url.unwrap_or_default(),
            owner_id: Uuid::parse_str(&result.owner_id).unwrap(),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
    
    // 更新群组信息
    pub async fn update_group(&self, group_id: Uuid, name: Option<String>, 
                            description: Option<String>, avatar_url: Option<String>) -> Result<Group> {
        let now = Utc::now();
        let now_naive = now.naive_utc();
        
        // 先获取现有数据
        let current = self.get_group(group_id).await?;
        
        // 更新群组信息
        let result = sqlx::query!(
            r#"
            UPDATE groups
            SET name = $1, description = $2, avatar_url = $3, updated_at = $4
            WHERE id = $5
            RETURNING id, name, description, avatar_url, owner_id, created_at, updated_at
            "#,
            name.unwrap_or(current.name),
            description.unwrap_or(current.description),
            avatar_url.unwrap_or(current.avatar_url),
            now_naive,
            group_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Group {
            id: Uuid::parse_str(&result.id).unwrap(),
            name: result.name,
            description: result.description.unwrap_or_default(),
            avatar_url: result.avatar_url.unwrap_or_default(),
            owner_id: Uuid::parse_str(&result.owner_id).unwrap(),
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
    
    // 删除群组
    pub async fn delete_group(&self, group_id: Uuid, user_id: Uuid) -> Result<bool> {
        // 先检查是否是群主
        let group = self.get_group(group_id).await?;
        if group.owner_id != user_id {
            return Err(anyhow::anyhow!("只有群主可以删除群组"));
        }
        
        let rows_affected = sqlx::query!(
            r#"
            DELETE FROM groups
            WHERE id = $1
            "#,
            group_id.to_string()
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        
        Ok(rows_affected > 0)
    }
    
    // 获取群组成员数量
    pub async fn get_member_count(&self, group_id: Uuid) -> Result<i32> {
        let result = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM group_members
            WHERE group_id = $1
            "#,
            group_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(result.count.unwrap_or(0) as i32)
    }
    
    // 获取用户加入的群组列表
    pub async fn get_user_groups(&self, user_id: Uuid) -> Result<Vec<UserGroup>> {
        let groups = sqlx::query!(
            r#"
            SELECT 
                g.id,
                g.name,
                g.avatar_url,
                m.role,
                m.joined_at,
                (SELECT COUNT(*) FROM group_members WHERE group_id = g.id) as member_count
            FROM groups g
            JOIN group_members m ON g.id = m.group_id
            WHERE m.user_id = $1
            "#,
            user_id.to_string()
        )
        .fetch_all(&self.pool)
        .await?;
        
        let result = groups
            .into_iter()
            .map(|g| UserGroup {
                id: Uuid::parse_str(&g.id).unwrap(),
                name: g.name,
                avatar_url: g.avatar_url.unwrap_or_default(),
                member_count: g.member_count.unwrap_or(0) as i32,
                role: g.role.parse::<i32>().unwrap_or(0),
                joined_at: Utc.from_utc_datetime(&g.joined_at),
            })
            .collect();
        
        Ok(result)
    }
}