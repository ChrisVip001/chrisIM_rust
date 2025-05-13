use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use common::proto::friend::FriendshipStatus;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friendship {
    pub id: Uuid,
    pub user_id: Uuid,
    pub friend_id: Uuid,
    pub status: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Friendship {
    pub fn new(user_id: Uuid, friend_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            friend_id,
            status: FriendshipStatus::Pending as i32,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
    
    pub fn to_proto(&self) -> common::proto::friend::Friendship {
        let created_system_time = SystemTime::from(self.created_at);
        let updated_system_time = SystemTime::from(self.updated_at);
        
        common::proto::friend::Friendship {
            id: self.id.to_string(),
            user_id: self.user_id.to_string(),
            friend_id: self.friend_id.to_string(),
            status: self.status,
            created_at: Some(prost_types::Timestamp::from(created_system_time)),
            updated_at: Some(prost_types::Timestamp::from(updated_system_time)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friend {
    pub id: Uuid,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub friendship_created_at: DateTime<Utc>,
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
        }
    }
}