use crate::error::Error;
use crate::proto::friend::{
    DeleteFriendRequest, Friend, FriendDb, Friendship, FriendshipStatus, FriendshipWithUser
};
use crate::proto::user::User;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, Row};
use std::fmt::{Display, Formatter};
use chrono::{DateTime, Utc};
use prost_types::Timestamp;

impl Display for FriendshipStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            FriendshipStatus::Pending => write!(f, "Pending"),
            FriendshipStatus::Accepted => write!(f, "Accepted"),
            FriendshipStatus::Rejected => write!(f, "Rejected"),
            FriendshipStatus::Blocked => write!(f, "Blocked"),
            FriendshipStatus::Expired => write!(f, "Expired"),
        }
    }
}
#[derive(sqlx::Type, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[sqlx(type_name = "friend_request_status")]
pub enum FsStatus {
    #[default]
    Pending,
    Accepted,
    Rejected,
    /// blacklist
    Blocked,
    Expired,
}

impl From<FsStatus> for FriendshipStatus {
    fn from(value: FsStatus) -> Self {
        match value {
            FsStatus::Pending => Self::Pending,
            FsStatus::Accepted => Self::Accepted,
            FsStatus::Rejected => Self::Rejected,
            FsStatus::Blocked => Self::Blocked,
            FsStatus::Expired => Self::Expired,
        }
    }
}

impl FromRow<'_, PgRow> for Friendship {
    fn from_row(row: &'_ PgRow) -> Result<Self, sqlx::Error> {
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();
        let updated_at: Option<DateTime<Utc>> = row.try_get("updated_at").ok();

        Ok(Self {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            friend_id: row.try_get("friend_id")?,
            status: row.try_get("status")?,
            created_at: created_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            updated_at: updated_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            message: row.try_get("message").unwrap_or_default(),
            reject_reason: row.try_get("reject_reason").ok(),
            friend_username: row.try_get("friend_username").ok(),
            friend_nickname: row.try_get("friend_nickname").ok(),
            friend_avatar_url: row.try_get("friend_avatar_url").ok(),
            friend_remark: row.try_get("friend_remark").ok(),
        })
    }
}

impl FromRow<'_, PgRow> for FriendDb {
    fn from_row(row: &'_ PgRow) -> Result<Self, sqlx::Error> {
        let status: FsStatus = row.try_get("status")?;
        let status = FriendshipStatus::from(status);
        Ok(Self {
            id: row.try_get("id")?,
            fs_id: row.try_get("fs_id")?,
            user_id: row.try_get("user_id")?,
            friend_id: row.try_get("friend_id")?,
            status: status as i32,
            remark: row.try_get("remark")?,
            source: row.try_get("source")?,
            create_time: row.try_get("create_time")?,
            update_time: row.try_get("update_time")?,
        })
    }
}

impl FromRow<'_, PgRow> for Friend {
    fn from_row(row: &'_ PgRow) -> Result<Self, sqlx::Error> {
        let friendship_created_at: Option<DateTime<Utc>> = row.try_get("friendship_created_at").ok();

        Ok(Self {
            id: row.try_get("id").unwrap_or_default(),
            remark: row.try_get("remark").unwrap_or_default(),
            username: row.try_get("username").ok(),
            nickname: row.try_get("nickname").ok(),
            avatar_url: row.try_get("avatar_url").ok(),
            friendship_created_at: friendship_created_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
        })
    }
}

impl From<User> for FriendshipWithUser {
    fn from(value: User) -> Self {
        Self {
            fs_id: String::new(),
            user_id: value.id,
            name: value.username,
            avatar: value.avatar_url.unwrap_or_default(),
            gender: String::new(), // User中没有gender字段
            age: 0, // User中没有age字段
            region: None,
            status: 0,
            apply_msg: None,
            source: String::new(),
            create_time: 0,
            account: String::new(), // User中没有account字段
            remark: None,
            email: Option::from(value.email),
        }
    }
}

impl FromRow<'_, PgRow> for FriendshipWithUser {
    fn from_row(row: &'_ PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            fs_id: row.try_get("fs_id").unwrap_or_default(),
            user_id: row.try_get("user_id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            account: row.try_get("account").unwrap_or_default(),
            avatar: row.try_get("avatar").unwrap_or_default(),
            gender: row.try_get("gender").unwrap_or_default(),
            age: row.try_get("age").unwrap_or_default(),
            region: row.try_get("region").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_default(),
            apply_msg: row.try_get("apply_msg").unwrap_or_default(),
            source: row.try_get("source").unwrap_or_default(),
            create_time: row.try_get("create_time").unwrap_or_default(),
            email: row.try_get("email").unwrap_or_default(),
            remark: None,
        })
    }
}

impl From<User> for Friend {
    fn from(value: User) -> Self {
        Self {
            id: String::new(),
            remark: Option::from(String::new()),
            username: Option::from(value.username),
            nickname: value.nickname,
            avatar_url: value.avatar_url,
            friendship_created_at: None,
        }
    }
}

impl DeleteFriendRequest {
    pub fn validate(&self) -> Result<(), Error> {
        if self.user_id.is_empty() {
            return Err(Error::BadRequest("user id is none".to_string()));
        }

        if self.friend_id.is_empty() {
            return Err(Error::BadRequest("friend id is none".to_string()));
        }

        Ok(())
    }
}
