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
        allow_member_friendship: Option<bool>,
        join_approval_required: Option<bool>,
        only_admin_can_invite: Option<bool>,
        only_admin_can_modify: Option<bool>,
        notify_member_join: Option<bool>,
        all_member_muted: Option<bool>,
    ) -> Result<GroupSettings> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 先检查是否已存在设置
        let exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(SELECT 1 FROM group_settings WHERE group_id = $1) as "exists!"
            "#,
            group_id
        )
        .fetch_one(&self.pool)
        .await?;

        if exists {
            // 获取现有设置
            let current = sqlx::query!(
                r#"
                SELECT allow_member_friendship, join_approval_required, only_admin_can_invite, only_admin_can_modify,notify_member_join,all_member_muted
                FROM group_settings
                WHERE group_id = $1
                "#,
                group_id
            )
            .fetch_one(&self.pool)
            .await?;

            // 构建更新SQL，只更新有值的字段
            let mut query_builder = sqlx::QueryBuilder::new(
                "UPDATE group_settings SET updated_at = "
            );
            
            query_builder.push_bind(now_naive);
            
            let mut separated = query_builder.separated(", ");
            
            if let Some(allow) = allow_member_friendship {
                separated.push("allow_member_friendship = ");
                separated.push_bind(if allow { 1 } else { 0 });
            }
            
            if let Some(approval) = join_approval_required {
                separated.push("join_approval_required = ");
                separated.push_bind(if approval { 1 } else { 0 });
            }
            
            if let Some(admin_invite) = only_admin_can_invite {
                separated.push("only_admin_can_invite = ");
                separated.push_bind(if admin_invite { 1 } else { 0 });
            }
            
            if let Some(admin_modify) = only_admin_can_modify {
                separated.push("only_admin_can_modify = ");
                separated.push_bind(if admin_modify { 1 } else { 0 });
            }
            
            if let Some(notify) = notify_member_join {
                separated.push("notify_member_join = ");
                separated.push_bind(if notify { 1 } else { 0 });
            }
            
            if let Some(muted) = all_member_muted {
                separated.push("all_member_muted = ");
                separated.push_bind(if muted { 1 } else { 0 });
            }
            
            query_builder.push(" WHERE group_id = ");
            query_builder.push_bind(&group_id);
            
            query_builder.build().execute(&self.pool).await?;

            Ok(GroupSettings {
                group_id,
                allow_member_friendship: allow_member_friendship.unwrap_or(current.allow_member_friendship != 0),
                join_approval_required: join_approval_required.unwrap_or(current.join_approval_required != 0),
                only_admin_can_invite: only_admin_can_invite.unwrap_or(current.only_admin_can_invite != 0),
                only_admin_can_modify: only_admin_can_modify.unwrap_or(current.only_admin_can_modify != 0),
                notify_member_join: notify_member_join.unwrap_or(current.notify_member_join != 0),
                all_member_muted: all_member_muted.unwrap_or(current.all_member_muted != 0),
                updated_at: now,
            })
        } else {
            // 创建新设置，使用默认值或传入的值
            let default = GroupSettings::new(group_id.clone());
            let settings = GroupSettings {
                group_id,
                allow_member_friendship: allow_member_friendship.unwrap_or(default.allow_member_friendship),
                join_approval_required: join_approval_required.unwrap_or(default.join_approval_required),
                only_admin_can_invite: only_admin_can_invite.unwrap_or(default.only_admin_can_invite),
                only_admin_can_modify: only_admin_can_modify.unwrap_or(default.only_admin_can_modify),
                notify_member_join: notify_member_join.unwrap_or(default.notify_member_join),
                all_member_muted: all_member_muted.unwrap_or(default.all_member_muted),
                updated_at: now,
            };

            self.create_group_settings(settings).await
        }
    }
} 