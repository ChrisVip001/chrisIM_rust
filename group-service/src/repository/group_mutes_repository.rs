use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;

use crate::model::group_mute::GroupMuteEntry;

pub struct GroupMutesRepository {
    pool: PgPool,
}

impl GroupMutesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 禁言成员
    pub async fn mute_member(
        &self,
        group_id: String,
        user_id: String,
        creator_id: String,
        reason: Option<String>,
        mute_until: Option<chrono::DateTime<Utc>>,
        is_permanent: bool,
    ) -> Result<GroupMuteEntry> {
        // 检查是否已经存在禁言记录
        let existing = sqlx::query!(
            r#"
            SELECT id FROM group_mutes
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;

        // 创建禁言记录
        let entry = GroupMuteEntry::new(group_id, user_id, creator_id, reason, mute_until, is_permanent);
        let created_at_naive = entry.created_at.naive_utc();
        let updated_at_naive = entry.updated_at.naive_utc();
        let mute_until_naive = entry.mute_until.map(|dt| dt.naive_utc());

        if existing.is_some() {
            // 更新现有禁言记录
            sqlx::query!(
                r#"
                UPDATE group_mutes
                SET creator_id = $1, reason = $2, mute_until = $3, is_permanent = $4, updated_at = $5
                WHERE group_id = $6 AND user_id = $7
                RETURNING id
                "#,
                entry.creator_id,
                entry.reason,
                mute_until_naive,
                entry.is_permanent as i32,
                updated_at_naive,
                entry.group_id,
                entry.user_id
            )
            .fetch_one(&self.pool)
            .await?;
        } else {
            // 创建新禁言记录
            sqlx::query!(
                r#"
                INSERT INTO group_mutes 
                (id, group_id, user_id, creator_id, reason, mute_until, is_permanent, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
                entry.id,
                entry.group_id,
                entry.user_id,
                entry.creator_id,
                entry.reason,
                mute_until_naive,
                entry.is_permanent as i32,
                created_at_naive,
                updated_at_naive
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(entry)
    }

    // 解除成员禁言
    pub async fn unmute_member(
        &self,
        group_id: String,
        user_id: String,
    ) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            DELETE FROM group_mutes
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // 获取被禁言的成员列表
    pub async fn get_muted_members(&self, group_id: String) -> Result<Vec<GroupMuteEntry>> {
        let results = sqlx::query!(
            r#"
            SELECT id, group_id, user_id, creator_id, reason, mute_until, is_permanent, created_at, updated_at
            FROM group_mutes
            WHERE group_id = $1
            ORDER BY created_at DESC
            "#,
            group_id
        )
        .fetch_all(&self.pool)
        .await?;

        let entries = results
            .into_iter()
            .map(|row| GroupMuteEntry {
                id: row.id,
                group_id: row.group_id,
                user_id: row.user_id,
                creator_id: row.creator_id,
                reason: row.reason,
                mute_until: row.mute_until.map(|dt| Utc.from_utc_datetime(&dt)),
                is_permanent: row.is_permanent != 0,
                created_at: Utc.from_utc_datetime(&row.created_at),
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            })
            .collect();

        Ok(entries)
    }

    // 获取用户禁言状态
    pub async fn get_mute_status(&self, group_id: String, user_id: String) -> Result<Option<GroupMuteEntry>> {
        let result = sqlx::query!(
            r#"
            SELECT id, group_id, user_id, creator_id, reason, mute_until, is_permanent, created_at, updated_at
            FROM group_mutes
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;

        let entry = result.map(|row| GroupMuteEntry {
            id: row.id,
            group_id: row.group_id,
            user_id: row.user_id,
            creator_id: row.creator_id,
            reason: row.reason,
            mute_until: row.mute_until.map(|dt| Utc.from_utc_datetime(&dt)),
            is_permanent: row.is_permanent != 0,
            created_at: Utc.from_utc_datetime(&row.created_at),
            updated_at: Utc.from_utc_datetime(&row.updated_at),
        });

        Ok(entry)
    }

    // 检查用户是否被禁言
    pub async fn is_user_muted(&self, group_id: String, user_id: String) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT is_permanent, mute_until 
            FROM group_mutes
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = result {
            let is_permanent = row.is_permanent != 0;
            if is_permanent {
                return Ok(true);
            }

            if let Some(mute_until) = row.mute_until {
                let now = Utc::now().naive_utc();
                return Ok(mute_until > now);
            }
        }

        Ok(false)
    }

    // 清理过期的禁言记录
    pub async fn clean_expired_mutes(&self) -> Result<u64> {
        let now = Utc::now().naive_utc();

        let result = sqlx::query!(
            r#"
            DELETE FROM group_mutes
            WHERE is_permanent = 0 AND mute_until IS NOT NULL AND mute_until <= $1
            "#,
            now
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // 获取群组中所有被禁言的成员的状态（仅返回当前有效的禁言）
    pub async fn get_active_mutes_by_group_id(&self, group_id: String) -> Result<std::collections::HashMap<String, GroupMuteEntry>> {
        let now = Utc::now().naive_utc();
        
        let results = sqlx::query!(
            r#"
            SELECT id, group_id, user_id, creator_id, reason, mute_until, is_permanent, created_at, updated_at
            FROM group_mutes
            WHERE group_id = $1 AND (is_permanent = 1 OR (mute_until IS NOT NULL AND mute_until > $2))
            "#,
            group_id,
            now
        )
        .fetch_all(&self.pool)
        .await?;

        let mut mute_map = std::collections::HashMap::new();
        for row in results {
            let entry = GroupMuteEntry {
                id: row.id,
                group_id: row.group_id,
                user_id: row.user_id.clone(),
                creator_id: row.creator_id,
                reason: row.reason,
                mute_until: row.mute_until.map(|dt| Utc.from_utc_datetime(&dt)),
                is_permanent: row.is_permanent != 0,
                created_at: Utc.from_utc_datetime(&row.created_at),
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            };
            
            mute_map.insert(row.user_id, entry);
        }

        Ok(mute_map)
    }
} 