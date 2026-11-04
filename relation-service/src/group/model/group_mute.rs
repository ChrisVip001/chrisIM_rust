use chrono::{DateTime, Utc};
use prost_types;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMuteEntry {
    pub id: String,
    pub group_id: String,
    pub user_id: String,
    pub creator_id: String,
    pub reason: Option<String>,
    pub mute_until: Option<DateTime<Utc>>,
    pub is_permanent: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GroupMuteEntry {
    pub fn new(
        group_id: String,
        user_id: String,
        creator_id: String,
        reason: Option<String>,
        mute_until: Option<DateTime<Utc>>,
        is_permanent: bool,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            group_id,
            user_id,
            creator_id,
            reason,
            mute_until,
            is_permanent,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn to_proto(&self) -> common::proto::group::MuteEntry {
        let created_system_time = SystemTime::from(self.created_at);
        let updated_system_time = SystemTime::from(self.updated_at);
        let mute_until_system_time = self.mute_until.map(SystemTime::from);

        common::proto::group::MuteEntry {
            id: self.id.clone(),
            group_id: self.group_id.clone(),
            user_id: self.user_id.clone(),
            creator_id: self.creator_id.clone(),
            reason: self.reason.clone().unwrap_or_default(),
            mute_until: mute_until_system_time.map(prost_types::Timestamp::from),
            is_permanent: self.is_permanent,
            created_at: Some(prost_types::Timestamp::from(created_system_time)),
            updated_at: Some(prost_types::Timestamp::from(updated_system_time)),
        }
    }

    // 检查禁言是否有效
    pub fn is_active(&self) -> bool {
        if self.is_permanent {
            return true;
        }

        if let Some(mute_until) = self.mute_until {
            return mute_until > Utc::now();
        }

        false
    }
} 