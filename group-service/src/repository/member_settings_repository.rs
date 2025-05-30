use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::model::member_settings::MemberSettings;

pub struct MemberSettingsRepository {
    pool: PgPool,
}

impl MemberSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 获取成员设置
    pub async fn get_member_settings(
        &self,
        group_id: String,
        user_id: String,
    ) -> Result<MemberSettings> {
        // 检查是否存在设置
        let exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(SELECT 1 FROM group_member_settings WHERE group_id = $1 AND user_id = $2) as "exists!"
            "#,
            group_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        if exists {
            // 获取现有设置
            let row = sqlx::query!(
                r#"
                SELECT id, group_id, user_id, mute_notifications, nickname_in_group, updated_at
                FROM group_member_settings
                WHERE group_id = $1 AND user_id = $2
                "#,
                group_id,
                user_id
            )
            .fetch_one(&self.pool)
            .await?;

            Ok(MemberSettings {
                id: row.id,
                group_id: row.group_id,
                user_id: row.user_id,
                mute_notifications: row.mute_notifications != 0,
                nickname_in_group: row.nickname_in_group.unwrap_or_default(),
                created_at: Utc::now(), // 简化处理，使用当前时间
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            })
        } else {
            // 创建新设置
            let default_settings = MemberSettings::new(group_id.clone(), user_id.clone());
            self.create_member_settings(default_settings).await
        }
    }

    // 创建成员设置
    pub async fn create_member_settings(&self, settings: MemberSettings) -> Result<MemberSettings> {
        let updated_at = settings.updated_at.naive_utc();

        sqlx::query!(
            r#"
            INSERT INTO group_member_settings 
            (id, group_id, user_id, mute_notifications, nickname_in_group, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            settings.id,
            settings.group_id,
            settings.user_id,
            settings.mute_notifications as i32,
            settings.nickname_in_group,
            updated_at
        )
        .execute(&self.pool)
        .await?;

        Ok(settings)
    }

    // 更新成员设置
    pub async fn update_member_settings(
        &self,
        group_id: String,
        user_id: String,
        mute_notifications: bool,
        nickname_in_group: String,
    ) -> Result<MemberSettings> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 检查是否已存在设置
        let exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(SELECT 1 FROM group_member_settings WHERE group_id = $1 AND user_id = $2) as "exists!"
            "#,
            group_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        if exists {
            // 更新现有设置
            let result = sqlx::query!(
                r#"
                UPDATE group_member_settings
                SET mute_notifications = $1, nickname_in_group = $2, updated_at = $3
                WHERE group_id = $4 AND user_id = $5
                RETURNING id
                "#,
                mute_notifications as i32,
                nickname_in_group,
                now_naive,
                group_id,
                user_id
            )
            .fetch_one(&self.pool)
            .await?;

            Ok(MemberSettings {
                id: result.id,
                group_id,
                user_id,
                mute_notifications,
                nickname_in_group,
                created_at: now, // 简化处理，使用当前时间
                updated_at: now,
            })
        } else {
            // 创建新设置
            let settings = MemberSettings {
                id: Uuid::new_v4().to_string(),
                group_id,
                user_id,
                mute_notifications,
                nickname_in_group,
                created_at: now,
                updated_at: now,
            };

            self.create_member_settings(settings).await
        }
    }
} 