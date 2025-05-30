use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;

use crate::model::group_blacklist::GroupBlacklistEntry;

pub struct GroupBlacklistRepository {
    pool: PgPool,
}

impl GroupBlacklistRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 添加用户到黑名单
    pub async fn add_to_blacklist(
        &self,
        group_id: String,
        user_id: String,
        creator_id: String,
        reason: Option<String>,
    ) -> Result<GroupBlacklistEntry> {
        // 首先检查用户是否已经在黑名单中
        let existing = sqlx::query!(
            r#"
            SELECT id FROM group_blacklist
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;

        if existing.is_some() {
            return Err(anyhow::anyhow!("该用户已经在黑名单中"));
        }

        // 创建黑名单条目
        let entry = GroupBlacklistEntry::new(group_id, user_id, creator_id, reason);
        let created_at_naive = entry.created_at.naive_utc();

        sqlx::query!(
            r#"
            INSERT INTO group_blacklist (id, group_id, user_id, creator_id, reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            entry.id,
            entry.group_id,
            entry.user_id,
            entry.creator_id,
            entry.reason,
            created_at_naive
        )
        .execute(&self.pool)
        .await?;

        // 如果是群成员，则从群组中移除
        sqlx::query!(
            r#"
            DELETE FROM group_members
            WHERE group_id = $1 AND user_id = $2
            "#,
            entry.group_id,
            entry.user_id
        )
        .execute(&self.pool)
        .await?;

        Ok(entry)
    }

    // 从黑名单中移除用户
    pub async fn remove_from_blacklist(
        &self,
        group_id: String,
        user_id: String,
    ) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            DELETE FROM group_blacklist
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // 获取群组黑名单
    pub async fn get_blacklist(&self, group_id: String) -> Result<Vec<GroupBlacklistEntry>> {
        let results = sqlx::query!(
            r#"
            SELECT id, group_id, user_id, creator_id, reason, created_at
            FROM group_blacklist
            WHERE group_id = $1
            ORDER BY created_at DESC
            "#,
            group_id
        )
        .fetch_all(&self.pool)
        .await?;

        let entries = results
            .into_iter()
            .map(|row| GroupBlacklistEntry {
                id: row.id,
                group_id: row.group_id,
                user_id: row.user_id,
                creator_id: row.creator_id,
                reason: row.reason,
                created_at: Utc.from_utc_datetime(&row.created_at),
            })
            .collect();

        Ok(entries)
    }

    // 检查用户是否在黑名单中
    pub async fn is_user_blacklisted(&self, group_id: String, user_id: String) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS(SELECT 1 FROM group_blacklist WHERE group_id = $1 AND user_id = $2) AS exists
            "#,
            group_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result.exists.unwrap_or(false))
    }

    // 获取黑名单条目
    pub async fn get_blacklist_entry(
        &self,
        group_id: String,
        user_id: String,
    ) -> Result<Option<GroupBlacklistEntry>> {
        let result = sqlx::query!(
            r#"
            SELECT id, group_id, user_id, creator_id, reason, created_at
            FROM group_blacklist
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|row| GroupBlacklistEntry {
            id: row.id,
            group_id: row.group_id,
            user_id: row.user_id,
            creator_id: row.creator_id,
            reason: row.reason,
            created_at: Utc.from_utc_datetime(&row.created_at),
        }))
    }
} 