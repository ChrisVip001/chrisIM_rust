use chrono::{DateTime, Utc};
use common::proto::friend::{Friendship as ProtoFriendship, FriendshipStatus};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgRow, FromRow, Row};
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friendship {
    pub id: Uuid,
    pub user_id: Uuid,
    pub friend_id: Uuid,
    pub message: String,
    pub status: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reject_reason: Option<String>,
}

impl Friendship {
    pub fn new(user_id: Uuid, friend_id: Uuid, message: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id,
            friend_id,
            message,
            status: FriendshipStatus::Pending as i32,
            created_at: now,
            updated_at: now,
            reject_reason: None,
        }
    }

    pub fn to_proto(&self) -> ProtoFriendship {
        ProtoFriendship {
            id: self.id.to_string(),
            user_id: self.user_id.to_string(),
            friend_id: self.friend_id.to_string(),
            message: self.message.clone(),
            status: self.status,
            created_at: Some(prost_types::Timestamp::from(SystemTime::from(self.created_at))),
            updated_at: Some(prost_types::Timestamp::from(SystemTime::from(self.updated_at))),
            reject_reason: self.reject_reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Friend {
    pub id: Uuid,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub friendship_created_at: DateTime<Utc>,
    pub remark: Option<String>,
}

impl Friend {
    pub fn to_proto(&self) -> common::proto::friend::Friend {
        let created_system_time = SystemTime::from(self.friendship_created_at);

        common::proto::friend::Friend {
            id: self.id.to_string(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            friendship_created_at: Some(prost_types::Timestamp::from(created_system_time)),
            remark: self.remark.clone(),
        }
    }
}
