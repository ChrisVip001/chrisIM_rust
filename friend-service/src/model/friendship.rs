use chrono::{DateTime, Utc};
use common::proto::friend::{Friendship as ProtoFriendship, FriendshipStatus, PotentialFriend as ProtoPotentialFriend};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgRow, FromRow, Row};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Friendship {
    pub id: String,
    pub user_id: String,
    pub friend_id: String,
    pub message: String,
    pub status: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reject_reason: Option<String>,
    pub friend_username: Option<String>,
    pub friend_nickname: Option<String>,
    pub friend_avatar_url: Option<String>,
}

impl Friendship {
    pub fn new(user_id: String, friend_id: String, message: String) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            friend_id,
            message,
            status: 0, // PENDING
            created_at: now,
            updated_at: now,
            reject_reason: None,
            friend_username: None,
            friend_nickname: None,
            friend_avatar_url: None,
        }
    }

    pub fn to_proto(&self) -> ProtoFriendship {
        ProtoFriendship {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            friend_id: self.friend_id.clone(),
            message: self.message.clone(),
            status: self.status,
            created_at: Some(prost_types::Timestamp::from(SystemTime::from(self.created_at))),
            updated_at: Some(prost_types::Timestamp::from(SystemTime::from(self.updated_at))),
            reject_reason: self.reject_reason.clone(),
            friend_username: self.friend_username.clone(),
            friend_nickname: self.friend_nickname.clone(),
            friend_avatar_url: self.friend_avatar_url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Friend {
    pub id: String,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub friendship_created_at: DateTime<Utc>,
    pub remark: Option<String>,
}

impl Friend {
    pub fn to_proto(&self) -> common::proto::friend::Friend {
        let created_system_time = SystemTime::from(self.friendship_created_at);

        common::proto::friend::Friend {
            id: self.id.clone(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            friendship_created_at: Some(prost_types::Timestamp::from(created_system_time)),
            remark: self.remark.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FriendGroup {
    pub id: String,
    pub user_id: String,
    pub group_name: String,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub friend_count: i32,
}

impl FriendGroup {
    pub fn new(user_id: String, group_name: String, sort_order: i32) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            group_name,
            sort_order,
            created_at: now,
            updated_at: now,
            friend_count: 0,
        }
    }

    pub fn to_proto(&self) -> common::proto::friend::FriendGroup {
        common::proto::friend::FriendGroup {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            group_name: self.group_name.clone(),
            sort_order: self.sort_order,
            created_at: Some(prost_types::Timestamp::from(SystemTime::from(self.created_at))),
            updated_at: Some(prost_types::Timestamp::from(SystemTime::from(self.updated_at))),
            friend_count: self.friend_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PotentialFriend {
    pub id: String,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub friendship_status: i32,
}

impl PotentialFriend {
    pub fn to_proto(&self) -> ProtoPotentialFriend {
        ProtoPotentialFriend {
            id: self.id.clone(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            phone: self.phone.clone(),
            friendship_status: self.friendship_status,
        }
    }
    
    pub fn from_tuple(
        id: String,
        username: String,
        nickname: Option<String>,
        avatar_url: Option<String>,
        phone: Option<String>,
        friendship_status: i32,
    ) -> Self {
        Self {
            id,
            username,
            nickname,
            avatar_url,
            phone,
            friendship_status,
        }
    }
}
