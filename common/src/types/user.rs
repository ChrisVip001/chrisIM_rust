use crate::proto::user::{UserWithMatchType, User};
use sqlx::postgres::PgRow;
use sqlx::{Error, FromRow, Row};
use chrono::{DateTime, Utc};
use prost_types::Timestamp;

impl FromRow<'_, PgRow> for User {
    fn from_row(row: &'_ PgRow) -> Result<Self, Error> {
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: Option<DateTime<Utc>> = row.try_get("updated_at").ok();
        let last_login_time: Option<DateTime<Utc>> = row.try_get("last_login_time").ok();

        Ok(User {
            id: row.try_get("id")?,
            username: row.try_get("username")?,
            email: row.try_get("email")?,
            nickname: row.try_get("nickname")?,
            avatar_url: row.try_get("avatar")?,
            created_at: Some(Timestamp {
                seconds: created_at.timestamp(),
                nanos: created_at.timestamp_subsec_nanos() as i32,
            }),
            updated_at: updated_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            phone: row.try_get("phone")?,
            address: row.try_get("address").ok(),
            head_image: row.try_get("head_image").ok(),
            head_image_thumb: row.try_get("head_image_thumb").ok(),
            sex: row.try_get("sex").ok(),
            user_stat: row.try_get("user_stat").unwrap_or(1),
            tenant_id: row.try_get("tenant_id").unwrap_or_default(),
            last_login_time: last_login_time.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            custom_id: row.try_get("custom_id").unwrap_or_default(),
            sign: row.try_get("sign").ok(),
        })
    }
}
// impl FromRow<'_, PgRow> for UserWithMatchType {
//     fn from_row(row: &'_ PgRow) -> Result<Self, Error> {
//         Ok(UserWithMatchType {
//             id: row.try_get("id")?,
//             name: row.try_get("name")?,
//             account: row.try_get("account")?,
//             avatar: row.try_get("avatar")?,
//             gender: row.try_get("gender")?,
//             age: row.try_get("age")?,
//             email: row.try_get("email")?,
//             region: row.try_get("region")?,
//             birthday: row.try_get("birthday")?,
//             match_type: row.try_get("match_type")?,
//             signature: row.try_get("signature")?,
//             is_friend: false,
//         })
//     }
// }
