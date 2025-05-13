use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::time::SystemTime;
use prost_types;
use common::message::GroupMemSeq;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub avatar_url: String,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Group {
    pub fn new(name: String, description: String, avatar_url: String, owner_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            description,
            avatar_url,
            owner_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
    
    pub fn to_proto(&self, member_count: i32) -> common::proto::group::Group {
        let created_system_time = SystemTime::from(self.created_at);
        let updated_system_time = SystemTime::from(self.updated_at);
        
        common::proto::group::Group {
            id: self.id.to_string(),
            name: self.name.clone(),
            description: self.description.clone(),
            avatar_url: self.avatar_url.clone(),
            owner_id: self.owner_id.to_string(),
            member_count,
            created_at: Some(prost_types::Timestamp::from(created_system_time)),
            updated_at: Some(prost_types::Timestamp::from(updated_system_time)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGroup {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: String,
    pub member_count: i32,
    pub role: i32,
    pub joined_at: DateTime<Utc>,
}

impl UserGroup {
    pub fn to_proto(&self) -> common::proto::group::UserGroup {
        let joined_system_time = SystemTime::from(self.joined_at);
        
        common::proto::group::UserGroup {
            id: self.id.to_string(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
            member_count: self.member_count,
            role: self.role,
            joined_at: Some(prost_types::Timestamp::from(joined_system_time)),
        }
    }
}