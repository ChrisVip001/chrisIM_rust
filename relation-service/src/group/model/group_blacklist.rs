use chrono::{DateTime, Utc};
use prost_types;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupBlacklistEntry {
    pub id: String,
    pub group_id: String,
    pub user_id: String,
    pub creator_id: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl GroupBlacklistEntry {
    pub fn new(
        group_id: String,
        user_id: String,
        creator_id: String,
        reason: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            group_id,
            user_id,
            creator_id,
            reason,
            created_at: Utc::now(),
        }
    }

    pub fn to_proto(&self) -> common::proto::group::BlacklistEntry {
        let created_system_time = SystemTime::from(self.created_at);

        common::proto::group::BlacklistEntry {
            id: self.id.clone(),
            group_id: self.group_id.clone(),
            user_id: self.user_id.clone(),
            creator_id: self.creator_id.clone(),
            reason: self.reason.clone().unwrap_or_default(),
            created_at: Some(prost_types::Timestamp::from(created_system_time)),
        }
    }
} 