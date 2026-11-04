//! 用户配置只读查询
//!
//! 独立成一个轻量组件，供好友服务解析用户资料时使用。
//!
//! 如果把这项能力留在 `UserServiceImpl` 里，好友服务就会反向依赖用户服务，
//! 两者形成构造期循环 —— 历史上用 `Arc<OnceLock<_>>` 后绑定绕过，
//! 那是变通而不是解法。把「只查配置」这一个能力抽出来后，
//! 依赖关系变成单向：user → friend → user_config_query。

use contract::error::Error;
use contract::proto::user::{UserConfig, UserConfigResponse};
use sqlx::PgPool;

use crate::user::repository::user_config_repository::UserConfigRepository;

/// 用户配置只读查询
pub struct UserConfigQuery {
    repository: UserConfigRepository,
}

impl UserConfigQuery {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: UserConfigRepository::new(pool),
        }
    }

    /// 查询用户配置，未设置时由调用方决定默认值
    pub async fn get_user_config(&self, user_id: &str) -> Result<UserConfigResponse, Error> {
        let config = self.repository.get_user_config(user_id).await?;

        Ok(UserConfigResponse {
            user_config: Some(UserConfig {
                user_id: config.user_id,
                allow_phone_search: config.allow_phone_search,
                allow_id_search: config.allow_id_search,
                auto_load_video: config.auto_load_video,
                auto_load_pic: config.auto_load_pic,
                msg_read_flag: config.msg_read_flag,
                sound_enabled: config.sound_enabled,
                vibration_enabled: config.vibration_enabled,
                show_phone: config.show_phone,
                create_time: config.create_time.map(|dt| prost_types::Timestamp {
                    seconds: dt.timestamp(),
                    nanos: dt.timestamp_subsec_nanos() as i32,
                }),
                update_time: config.update_time.map(|dt| prost_types::Timestamp {
                    seconds: dt.timestamp(),
                    nanos: dt.timestamp_subsec_nanos() as i32,
                }),
            }),
        })
    }
}
