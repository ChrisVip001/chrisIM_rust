use chrono::{Utc};
use sqlx::{PgPool, QueryBuilder};
use common::{Result};
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
    pub async fn get_user_config(&self, id: &str) -> Result<UserConfig> {

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

        match row {
            Some(row) => {
                // 如果找到记录，返回用户配置
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
            None => {
                // 如果没有找到记录，返回默认配置
                Ok(UserConfig {
                    id: 0, // 使用默认值 0 作为占位符
                    user_id: id.to_string(),
                    allow_phone_search: Option::from(2),  // 设置默认值
                    allow_id_search: Option::from(2),     // 设置默认值
                    auto_load_video: Option::from(2),    // 设置默认值
                    auto_load_pic: Option::from(2),       // 设置默认值
                    msg_read_flag: Option::from(2),       // 设置默认值
                    create_time: Some(Utc::now()),
                    update_time: Some(Utc::now()),
                })
            }
        }
    }

    /// 保存用户设置
    pub async fn save_user_config(&self, data: &UserConfigData) -> Result<UserConfig> {

        // 检查用户设置是否存在
        let user_conifg_existed = self.get_user_config(&data.user_id).await;
        if user_conifg_existed?.id != 0 { // 检查 id 是否为默认值 0
            // 设置已存在则进行修改
            // 动态构建SET子句
            let mut builder = QueryBuilder::new(" UPDATE user_config SET ");
            let mut first = true;
            if let Some(allow_phone_search) = data.allow_phone_search {
                if !first { builder.push(","); }
                builder.push(" allow_phone_search = COALESCE(" ).push_bind(allow_phone_search).push(", allow_phone_search) ");
                first = false;
            }
            if let Some(allow_id_search) = data.allow_id_search {
                if !first { builder.push(","); }
                builder.push(" allow_id_search = COALESCE( ").push_bind(allow_id_search).push(", allow_id_search) ");
                first = false;
            }
            if let Some(auto_load_video) = data.auto_load_video {
                if !first { builder.push(","); }
                builder.push(" auto_load_video = COALESCE( ").push_bind(auto_load_video).push(", auto_load_video) ");
                first = false;
            }
            if let Some(auto_load_pic) = data.auto_load_pic {
                if !first { builder.push(","); }
                builder.push(" auto_load_pic = COALESCE( ").push_bind(auto_load_pic).push(", auto_load_pic) ");
                first = false;
            }
            if let Some(msg_read_flag) = data.msg_read_flag {
                if !first { builder.push(","); }
                builder.push(" msg_read_flag = COALESCE( ").push_bind(msg_read_flag).push(", msg_read_flag) ");
                first = false;
            }

            if !first { builder.push(","); }
            builder.push(" update_time = ").push_bind(Utc::now());
            builder.push(" WHERE user_id = ").push_bind(&data.user_id);
            builder.push(" RETURNING id, user_id, allow_phone_search, allow_id_search, auto_load_video, 
                auto_load_pic, msg_read_flag,create_time,update_time "
            );
            // 生成最终SQL
            let query = builder.build_query_as::<UserConfig>();
            let row = query.fetch_one(&self.pool).await?;
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
        } else {
            // 不存在则进行新增
            let row = sqlx::query!(
                r#"
                INSERT INTO user_config (user_id, allow_phone_search, allow_id_search, auto_load_video, 
                                         auto_load_pic,msg_read_flag)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING id, user_id, allow_phone_search, allow_id_search, auto_load_video, auto_load_pic, msg_read_flag,
                create_time,update_time
                "#,
                data.user_id,
                data.allow_phone_search,
                data.allow_id_search,
                data.auto_load_video,
                data.auto_load_pic,
                data.msg_read_flag,
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
}
