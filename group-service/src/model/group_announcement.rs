use chrono::{DateTime, Utc};
use prost_types;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupAnnouncement {
    pub id: String,
    pub group_id: String,
    pub creator_id: String,
    pub title: Option<String>,
    pub content: String,
    pub is_pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GroupAnnouncement {
    pub fn new(
        group_id: String,
        creator_id: String,
        title: Option<String>,
        content: String,
        is_pinned: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            group_id,
            creator_id,
            title,
            content,
            is_pinned,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn to_proto(&self) -> common::proto::group::Announcement {
        let created_system_time = SystemTime::from(self.created_at);
        let updated_system_time = SystemTime::from(self.updated_at);

        common::proto::group::Announcement {
            id: self.id.clone(),
            group_id: self.group_id.clone(),
            creator_id: self.creator_id.clone(),
            title: self.title.clone().unwrap_or_default(),
            content: self.content.clone(),
            is_pinned: self.is_pinned,
            created_at: Some(prost_types::Timestamp::from(created_system_time)),
            updated_at: Some(prost_types::Timestamp::from(updated_system_time)),
        }
    }
} 