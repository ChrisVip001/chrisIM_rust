use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use prost_types;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberSettings {
    pub id: String,
    pub group_id: String,
    pub user_id: String,
    pub mute_notifications: bool,
    pub nickname_in_group: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MemberSettings {
    pub fn new(group_id: String, user_id: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            group_id,
            user_id,
            mute_notifications: false,
            nickname_in_group: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn to_proto(&self) -> common::proto::group::MemberSettings {
        common::proto::group::MemberSettings {
            id: self.id.clone(),
            group_id: self.group_id.clone(),
            user_id: self.user_id.clone(),
            mute_notifications: self.mute_notifications,
            nickname_in_group: self.nickname_in_group.clone(),
            created_at: Some(prost_types::Timestamp {
                seconds: self.created_at.timestamp(),
                nanos: self.created_at.timestamp_subsec_nanos() as i32,
            }),
            updated_at: Some(prost_types::Timestamp {
                seconds: self.updated_at.timestamp(),
                nanos: self.updated_at.timestamp_subsec_nanos() as i32,
            }),
        }
    }
} 