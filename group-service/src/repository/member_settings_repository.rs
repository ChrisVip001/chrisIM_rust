use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use std::collections::HashMap;
use crate::model::group_mute::GroupMuteEntry;
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

    // 批量获取用户的群组成员设置
    pub async fn batch_get_member_settings(
        &self,
        user_id: &str,
        group_ids: &[String],
    ) -> Result<HashMap<String, MemberSettings>> {
        if group_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // 使用IN查询获取多个群组的设置
        let rows = sqlx::query!(
            r#"
            SELECT id, group_id, user_id, mute_notifications, nickname_in_group, updated_at, remark, is_top, recall_notification, show_nickname
            FROM group_member_settings
            WHERE user_id = $1 AND group_id = ANY($2)
            "#,
            user_id,
            &group_ids
        )
        .fetch_all(&self.pool)
        .await?;

        // 将结果转换为HashMap
        let mut settings_map = HashMap::new();
        for row in rows {
            let settings = MemberSettings {
                id: row.id,
                group_id: row.group_id.clone(),
                user_id: row.user_id,
                remark: row.remark.unwrap_or_default(),
                is_top: row.is_top != 0,
                recall_notification: row.recall_notification != 0,
                show_nickname: row.show_nickname != 0,
                mute_notifications: row.mute_notifications != 0,
                nickname_in_group: row.nickname_in_group.unwrap_or_default(),
                created_at: Utc::now(), // 简化处理，使用当前时间
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            };
            settings_map.insert(row.group_id, settings);
        }

        Ok(settings_map)
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

        let now_utc = Utc::now();
        let now = now_utc.naive_utc();

        let result = sqlx::query!(
            r#"
            UPDATE group_member_settings SET
            mute_notifications = COALESCE($3, mute_notifications),
            nickname_in_group = COALESCE($4, nickname_in_group),
            remark = COALESCE($5, remark),
            is_top = COALESCE($6, is_top),
            recall_notification = COALESCE($7, recall_notification),
            show_nickname = COALESCE($8, show_nickname),
                updated_at = $9
            WHERE group_id = $1 and user_id = $2
            RETURNING  group_id, user_id, mute_notifications, nickname_in_group, updated_at, remark, is_top, recall_notification, show_nickname
            "#,
            group_id,
            user_id,
            mute_notifications.map(|v| v as i32),
            nickname_in_group,
            remark,
            is_top.map(|v| v as i32),
            recall_notification.map(|v| v as i32),
            show_nickname.map(|v| v as i32),
            now
        )
            .fetch_optional(&self.pool)
            .await?;

        match result {
            Some(row) => Ok(MemberSettings {
                id: Uuid::new_v4().to_string(),
                group_id: row.group_id,
                user_id: row.user_id,
                remark: row.remark.unwrap_or_default(),
                is_top: row.is_top != 0,
                recall_notification: row.recall_notification != 0,
                show_nickname: row.show_nickname != 0,
                mute_notifications: row.mute_notifications != 0,
                updated_at: Utc.from_utc_datetime(&row.updated_at),
                nickname_in_group: row.nickname_in_group.unwrap_or_default(),
                created_at: now_utc
            }),
            None => {
                // 如果找不到设置，则创建默认设置并更新
                let mut default_settings = MemberSettings::new(group_id.clone(),  user_id.clone());

                // 应用提供的更新
                if let Some(val) = mute_notifications {
                    default_settings.mute_notifications = val;
                }
                if let Some(val) = nickname_in_group {
                    default_settings.nickname_in_group = val;
                }
                if let Some(val) = remark {
                    default_settings.remark = val;
                }
                if let Some(val) = is_top {
                    default_settings.is_top = val;
                }
                if let Some(val) = recall_notification {
                    default_settings.recall_notification = val;
                }
                if let Some(val) = show_nickname {
                    default_settings.show_nickname = val;
                }
                self.create_member_settings(default_settings).await
            }
        }
    }

    // 获取群组中所有成员的设置
    pub async fn get_active_mutes_by_group_id(&self, group_id: String) -> Result<std::collections::HashMap<String, MemberSettings>> {

        let rows = sqlx::query!(
            r#"
            SELECT group_id, user_id, mute_notifications, nickname_in_group, updated_at,created_at, remark, is_top, recall_notification, show_nickname
            FROM group_member_settings
            WHERE group_id = $1
            "#,
            group_id
        )
        .fetch_all(&self.pool)
        .await?;
        let mut settings_map = std::collections::HashMap::new();
        for row in rows {
            let settings = MemberSettings {
                id: Uuid::new_v4().to_string(),
                group_id: row.group_id,
                user_id: row.user_id.clone(),
                remark: row.remark.unwrap_or_default(),
                is_top: row.is_top != 0,
                recall_notification: row.recall_notification != 0,
                show_nickname: row.show_nickname != 0,
                mute_notifications: row.mute_notifications != 0,
                nickname_in_group: row.nickname_in_group.unwrap_or_default(),
                created_at: Utc.from_utc_datetime(&row.created_at),
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            };
            settings_map.insert(row.user_id, settings);
        }
        Ok(settings_map)
    }
} 