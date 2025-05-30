use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;

use crate::model::group_settings::GroupSettings;

pub struct GroupSettingsRepository {
    pool: PgPool,
}

impl GroupSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 获取群组设置
    pub async fn get_group_settings(&self, group_id: String) -> Result<GroupSettings> {
        let result = sqlx::query!(
            r#"
            SELECT group_id, allow_member_friendship, join_approval_required, 
                   only_admin_can_invite, only_admin_can_modify, updated_at
            FROM group_settings
            WHERE group_id = $1
            "#,
            group_id
        )
        .fetch_optional(&self.pool)
        .await?;

        // 如果找不到设置，则创建默认设置
        match result {
            Some(row) => Ok(GroupSettings {
                group_id: row.group_id,
                allow_member_friendship: row.allow_member_friendship != 0,
                join_approval_required: row.join_approval_required != 0,
                only_admin_can_invite: row.only_admin_can_invite != 0,
                only_admin_can_modify: row.only_admin_can_modify != 0,
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            }),
            None => {
                let default_settings = GroupSettings::new(group_id.clone());
                self.create_group_settings(default_settings).await
            }
        }
    }

    // 创建群组设置
    pub async fn create_group_settings(&self, settings: GroupSettings) -> Result<GroupSettings> {
        let updated_at_naive = settings.updated_at.naive_utc();

        sqlx::query!(
            r#"
            INSERT INTO group_settings (group_id, allow_member_friendship, join_approval_required, 
                                       only_admin_can_invite, only_admin_can_modify, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            settings.group_id,
            settings.allow_member_friendship as i32,
            settings.join_approval_required as i32,
            settings.only_admin_can_invite as i32,
            settings.only_admin_can_modify as i32,
            updated_at_naive
        )
        .execute(&self.pool)
        .await?;

        Ok(settings)
    }

    // 更新群组设置
    pub async fn update_group_settings(
        &self,
        group_id: String,
        allow_member_friendship: bool,
        join_approval_required: bool,
        only_admin_can_invite: bool,
        only_admin_can_modify: bool,
    ) -> Result<GroupSettings> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 先检查是否已存在设置
        let exists = sqlx::query!(
            r#"
            SELECT EXISTS(SELECT 1 FROM group_settings WHERE group_id = $1) AS exists
            "#,
            group_id
        )
        .fetch_one(&self.pool)
        .await?
        .exists
        .unwrap_or(false);

        if exists {
            // 更新现有设置
            sqlx::query!(
                r#"
                UPDATE group_settings
                SET allow_member_friendship = $1,
                    join_approval_required = $2,
                    only_admin_can_invite = $3,
                    only_admin_can_modify = $4,
                    updated_at = $5
                WHERE group_id = $6
                "#,
                allow_member_friendship as i32,
                join_approval_required as i32,
                only_admin_can_invite as i32,
                only_admin_can_modify as i32,
                now_naive,
                group_id
            )
            .execute(&self.pool)
            .await?;
        } else {
            // 创建新设置
            sqlx::query!(
                r#"
                INSERT INTO group_settings (group_id, allow_member_friendship, join_approval_required, 
                                           only_admin_can_invite, only_admin_can_modify, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
                group_id,
                allow_member_friendship as i32,
                join_approval_required as i32,
                only_admin_can_invite as i32,
                only_admin_can_modify as i32,
                now_naive
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(GroupSettings {
            group_id,
            allow_member_friendship,
            join_approval_required,
            only_admin_can_invite,
            only_admin_can_modify,
            updated_at: now,
        })
    }
} 