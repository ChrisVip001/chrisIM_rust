use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSettings {
    pub group_id: String,
    pub allow_member_friendship: bool,
    pub join_approval_required: bool,
    pub only_admin_can_invite: bool,
    pub only_admin_can_modify: bool,
    pub updated_at: DateTime<Utc>,
}

impl GroupSettings {
    pub fn new(group_id: String) -> Self {
        Self {
            group_id,
            allow_member_friendship: true,     // 默认允许群成员互加好友
            join_approval_required: false,     // 默认不需要审批加入
            only_admin_can_invite: false,      // 默认所有成员都可以邀请
            only_admin_can_modify: false,      // 默认所有成员都可以修改群信息
            updated_at: Utc::now(),
        }
    }

    // 转换为Proto对象
    pub fn to_proto(&self) -> common::proto::group::GroupSettings {
        common::proto::group::GroupSettings {
            group_id: self.group_id.clone(),
            allow_member_friendship: self.allow_member_friendship,
            join_approval_required: self.join_approval_required,
            only_admin_can_invite: self.only_admin_can_invite,
            only_admin_can_modify: self.only_admin_can_modify,
        }
    }
} 