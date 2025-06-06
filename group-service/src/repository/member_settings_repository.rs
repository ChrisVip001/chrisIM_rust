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
                SELECT id, group_id, user_id, mute_notifications, nickname_in_group, updated_at,remark,is_top,recall_notification,show_nickname
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
                remark: row.remark.unwrap_or_default(),
                is_top: row.is_top != 0,
                recall_notification: row.recall_notification != 0,
                show_nickname: row.show_nickname != 0,
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
            (id, group_id, user_id, mute_notifications, nickname_in_group, updated_at, remark, is_top, recall_notification, show_nickname)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
            settings.id,
            settings.group_id,
            settings.user_id,
            settings.mute_notifications as i32,
            settings.nickname_in_group,
            updated_at,
            settings.remark,
            settings.is_top as i32,
            settings.recall_notification as i32,
            settings.show_nickname as i32
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
        mute_notifications: Option<bool>,
        nickname_in_group: Option<String>,
        remark: Option<String>,
        is_top: Option<bool>,
        recall_notification: Option<bool>,
        show_nickname: Option<bool>,
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
            // 获取现有设置
            let current = sqlx::query!(
                r#"
                SELECT id, mute_notifications, nickname_in_group, remark, is_top, recall_notification, show_nickname
                FROM group_member_settings
                WHERE group_id = $1 AND user_id = $2
                "#,
                group_id,
                user_id
            )
            .fetch_one(&self.pool)
            .await?;

            // 构建更新SQL，只更新有值的字段
            let mut query_builder = sqlx::QueryBuilder::new(
                "UPDATE group_member_settings SET updated_at = "
            );
            
            query_builder.push_bind(now_naive);
            
            let mut separated = query_builder.separated(", ");
            
            if let Some(mute) = mute_notifications {
                separated.push("mute_notifications = ");
                separated.push_bind(if mute { 1 } else { 0 });
            }
            
            if let Some(nickname) = &nickname_in_group {
                separated.push("nickname_in_group = ");
                separated.push_bind(nickname);
            }
            
            if let Some(rem) = &remark {
                separated.push("remark = ");
                separated.push_bind(rem);
            }
            
            if let Some(top) = is_top {
                separated.push("is_top = ");
                separated.push_bind(if top { 1 } else { 0 });
            }
            
            if let Some(recall) = recall_notification {
                separated.push("recall_notification = ");
                separated.push_bind(if recall { 1 } else { 0 });
            }
            
            if let Some(show) = show_nickname {
                separated.push("show_nickname = ");
                separated.push_bind(if show { 1 } else { 0 });
            }
            
            query_builder.push(" WHERE group_id = ");
            query_builder.push_bind(&group_id);
            query_builder.push(" AND user_id = ");
            query_builder.push_bind(&user_id);
            
            query_builder.build().execute(&self.pool).await?;

            Ok(MemberSettings {
                id: current.id,
                group_id,
                user_id,
                mute_notifications: mute_notifications.unwrap_or(current.mute_notifications != 0),
                nickname_in_group: nickname_in_group.unwrap_or(current.nickname_in_group.unwrap_or_default()),
                remark: remark.unwrap_or(current.remark.unwrap_or_default()),
                is_top: is_top.unwrap_or(current.is_top != 0),
                recall_notification: recall_notification.unwrap_or(current.recall_notification != 0),
                show_nickname: show_nickname.unwrap_or(current.show_nickname != 0),
                created_at: now, // 简化处理，使用当前时间
                updated_at: now,
            })
        } else {
            // 创建新设置
            let settings = MemberSettings {
                id: Uuid::new_v4().to_string(),
                group_id,
                user_id,
                mute_notifications: mute_notifications.unwrap_or(false),
                nickname_in_group: nickname_in_group.unwrap_or_default(),
                remark: remark.unwrap_or_default(),
                is_top: is_top.unwrap_or(false),
                recall_notification: recall_notification.unwrap_or(false),
                show_nickname: show_nickname.unwrap_or(true),
                created_at: now,
                updated_at: now,
            };

            self.create_member_settings(settings).await
        }
    }
} 