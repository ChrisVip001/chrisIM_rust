use chrono::{TimeZone, Utc};
use sqlx::PgPool;
use common::{Error, Result};
use tracing::{debug, error};
use tracing::log::info;
use crate::model::user_config::{UserConfig, UserConfigData};

/// 用户设置仓库实现
pub struct UserConfigRepository {
    pool: PgPool,
}

impl UserConfigRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 查询用户设置
    pub async fn get_user_config(&self, id: &str) -> Result<Option<UserConfig>> {

        // 检查用户是否存在
        // let _user = self.get_user_by_id(id).await?;

        let row = sqlx::query!(
            r#"
            SELECT id, user_id, allow_phone_search, allow_id_search, auto_load_video, auto_load_pic, msg_read_flag,
                   create_time,update_time
            FROM user_config
            WHERE user_id = $1
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| UserConfig {
            id: row.id,
            user_id: row.user_id,
            allow_phone_search: row.allow_phone_search,
            allow_id_search: row.allow_id_search,
            auto_load_video: row.auto_load_video,
            auto_load_pic: row.auto_load_pic,
            msg_read_flag: row.msg_read_flag,
            create_time: row.create_time,
            update_time: row.update_time,
        }))
    }

    /// 保存用户设置
    pub async fn save_user_config(&self, data: &UserConfigData) -> Result<UserConfig> {

        // 检查用户是否存在
        // let _user = self.get_user_by_id(id).await?;

        let row = sqlx::query!(
            r#"
            SELECT id, user_id, allow_phone_search, allow_id_search, auto_load_video, auto_load_pic, msg_read_flag,
                   create_time,update_time
            FROM user_config
            WHERE user_id = $1
            "#,
            data.user_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(UserConfig {
            id: row.id,
            user_id: row.user_id,
            allow_phone_search: row.allow_phone_search,
            allow_id_search: row.allow_id_search,
            auto_load_video: row.auto_load_video,
            auto_load_pic: row.auto_load_pic,
            msg_read_flag: row.msg_read_flag,
            create_time: row.create_time,
            update_time: row.update_time,
        })
    }
}
