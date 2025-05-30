use chrono::{DateTime, Utc};
use common::proto::friend::UserBlacklist as ProtoUserBlacklist;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBlacklist {
    pub id: String,
    pub user_id: String,
    pub blocked_user_id: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl UserBlacklist {
    pub fn new(user_id: String, blocked_user_id: String, reason: Option<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            blocked_user_id,
            reason,
            created_at: Utc::now(),
        }
    }

    pub fn to_proto(&self) -> ProtoUserBlacklist {
        ProtoUserBlacklist {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            blocked_user_id: self.blocked_user_id.clone(),
            reason: self.reason.clone(),
            created_at: Some(prost_types::Timestamp::from(SystemTime::from(self.created_at))),
        }
    }
} 