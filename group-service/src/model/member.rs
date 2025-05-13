use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use common::proto::group::MemberRole;
use std::time::SystemTime;
use prost_types;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: Uuid,
    pub group_id: Uuid,
    pub user_id: Uuid,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub role: i32,
    pub joined_at: DateTime<Utc>,
}

impl Member {
    pub fn new(group_id: Uuid, user_id: Uuid, username: String, nickname: Option<String>, 
            avatar_url: Option<String>, role: MemberRole) -> Self {
        Self {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            username,
            nickname,
            avatar_url,
            role: role as i32,
            joined_at: Utc::now(),
        }
    }
    
    pub fn to_proto(&self) -> common::proto::group::Member {
        let joined_system_time = SystemTime::from(self.joined_at);
        
        common::proto::group::Member {
            id: self.id.to_string(),
            group_id: self.group_id.to_string(),
            user_id: self.user_id.to_string(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            role: self.role,
            joined_at: Some(prost_types::Timestamp::from(joined_system_time)),
        }
    }
}