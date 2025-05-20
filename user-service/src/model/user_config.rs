use chrono::{DateTime, Utc};
use common::proto::user;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// 用户设置数据库模型
#[derive(Debug, Clone, Serialize, Deserialize,FromRow)]
pub struct UserConfig {
    pub id: i32,
    pub user_id: String,
    pub allow_phone_search: Option<i32>,
    pub allow_id_search: Option<i32>,
    pub auto_load_video: Option<i32>,
    pub auto_load_pic: Option<i32>,
    pub msg_read_flag: Option<i32>,
    pub create_time: Option<DateTime<Utc>>,
    pub update_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfigData {
    pub user_id: String,
    pub allow_phone_search: Option<i32>,
    pub allow_id_search : Option<i32>,
    pub auto_load_video: Option<i32>,
    pub auto_load_pic: Option<i32>,
    pub msg_read_flag: Option<i32>,
}

impl From<user::UserConfigRequest> for UserConfigData {
    fn from(req: user::UserConfigRequest) -> Self {
        Self {
            user_id: req.user_id,
            allow_phone_search: req.allow_phone_search,
            allow_id_search: req.allow_id_search,
            auto_load_video: req.auto_load_video,
            auto_load_pic: req.auto_load_pic,
            msg_read_flag: req.msg_read_flag,
        }
    }
}
