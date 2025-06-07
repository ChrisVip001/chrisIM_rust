use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;

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
                   only_admin_can_invite, only_admin_can_modify, updated_at,notify_member_join,all_member_muted
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
                notify_member_join: row.notify_member_join != 0,  // 默认值
                all_member_muted: row.all_member_muted != 0,   // 默认值
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
                                       only_admin_can_invite, only_admin_can_modify, updated_at, all_member_muted,  notify_member_join)
            VALUES ($1, $2, $3, $4, $5, $6,  $7, $8)
            "#,
            settings.group_id,
            settings.allow_member_friendship as i32,
            settings.join_approval_required as i32,
            settings.only_admin_can_invite as i32,
            settings.only_admin_can_modify as i32,
            updated_at_naive,
            settings.all_member_muted as i32,
            settings.notify_member_join as i32
        )
        .execute(&self.pool)
        .await?;

        Ok(settings)
    }

    // 更新群组设置
    pub async fn update_group_settings(
        &self,
        group_id: String,
        allow_member_friendship: Option<bool>,
        join_approval_required: Option<bool>,
        only_admin_can_invite: Option<bool>,
        only_admin_can_modify: Option<bool>,
        notify_member_join: Option<bool>,
        all_member_muted: Option<bool>,
    ) -> Result<GroupSettings> {
        let now_utc = Utc::now();
        let now = now_utc.naive_utc();

        let result = sqlx::query!(
            r#"
            UPDATE group_settings SET
                allow_member_friendship = COALESCE($2, allow_member_friendship),
                join_approval_required = COALESCE($3, join_approval_required),
                only_admin_can_invite = COALESCE($4, only_admin_can_invite),
                only_admin_can_modify = COALESCE($5, only_admin_can_modify),
                notify_member_join = COALESCE($6, notify_member_join),
                all_member_muted = COALESCE($7, all_member_muted),
                updated_at = $8
            WHERE group_id = $1
            RETURNING  group_id, allow_member_friendship, join_approval_required,
                     only_admin_can_invite, only_admin_can_modify, updated_at, notify_member_join, all_member_muted
            "#,
            group_id,
            allow_member_friendship.map(|v| v as i32),
            join_approval_required.map(|v| v as i32),
            only_admin_can_invite.map(|v| v as i32),
            only_admin_can_modify.map(|v| v as i32),
            notify_member_join.map(|v| v as i32),
            all_member_muted.map(|v| v as i32),
            now
        )
        .fetch_optional(&self.pool)
        .await?;

        match result {
            Some(row) => Ok(GroupSettings {
                group_id: row.group_id,
                allow_member_friendship: row.allow_member_friendship != 0,
                join_approval_required: row.join_approval_required != 0,
                only_admin_can_invite: row.only_admin_can_invite != 0,
                only_admin_can_modify: row.only_admin_can_modify != 0,
                notify_member_join: row.notify_member_join != 0,
                all_member_muted: row.all_member_muted != 0,
                updated_at: Utc.from_utc_datetime(&row.updated_at),
            }),
            None => {
                // 如果找不到设置，则创建默认设置并更新
                let mut default_settings = GroupSettings::new(group_id.clone());
                
                // 应用提供的更新
                if let Some(val) = allow_member_friendship {
                    default_settings.allow_member_friendship = val;
                }
                if let Some(val) = join_approval_required {
                    default_settings.join_approval_required = val;
                }
                if let Some(val) = only_admin_can_invite {
                    default_settings.only_admin_can_invite = val;
                }
                if let Some(val) = only_admin_can_modify {
                    default_settings.only_admin_can_modify = val;
                }
                if let Some(val) = notify_member_join {
                    default_settings.notify_member_join = val;
                }
                if let Some(val) = all_member_muted {
                    default_settings.all_member_muted = val;
                }
                
                self.create_group_settings(default_settings).await
            }
        }
    }
} 