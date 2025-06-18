use sqlx::postgres::PgRow;
use sqlx::{Error, FromRow, Row};
use tonic::Status;
use chrono::{DateTime, Utc};
use prost_types::Timestamp;

use crate::proto::group::{
    GetGroupAndMembersResp, GetMemberReq, GroupInfo, Member, MemberRole,
    GroupMembersIdRequest, RemoveMemberRequest,
};
use super::Validator;

impl GroupMembersIdRequest {
    pub fn new(group_id: String) -> Self {
        Self { group_id }
    }
}

impl Validator for GetMemberReq {
    fn validate(&self) -> Result<(), Status> {
        if self.group_id.is_empty() {
            return Err(Status::invalid_argument("group_id is empty"));
        }
        if self.user_id.is_empty() {
            return Err(Status::invalid_argument("user_id is empty"));
        }
        if self.mem_ids.is_empty() {
            return Err(Status::invalid_argument("mem_ids is empty"));
        }
        Ok(())
    }
}

impl Validator for RemoveMemberRequest {
    fn validate(&self) -> Result<(), Status> {
        if self.group_id.is_empty() {
            return Err(Status::invalid_argument("group_id is empty"));
        }
        if self.user_ids.is_empty() {
            return Err(Status::invalid_argument("user_ids is empty"));
        }
        if self.removed_by_id.is_empty() {
            return Err(Status::invalid_argument("removed_by_id is empty"));
        }
        Ok(())
    }
}

impl GetGroupAndMembersResp {
    pub fn new(group: GroupInfo, members: Vec<Member>) -> Self {
        Self {
            group: Some(group),
            members,
        }
    }
}

#[derive(sqlx::Type, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[sqlx(type_name = "group_role")]
pub enum GroupRole {
    Owner,
    Admin,
    Member,
}

impl From<GroupRole> for MemberRole {
    fn from(value: GroupRole) -> Self {
        match value {
            GroupRole::Owner => Self::Owner,
            GroupRole::Admin => Self::Admin,
            GroupRole::Member => Self::Member,
        }
    }
}

// implement slqx FromRow trait
impl FromRow<'_, PgRow> for Member {
    fn from_row(row: &PgRow) -> Result<Self, Error> {
        let role: GroupRole = row.try_get("role")?;
        let role = MemberRole::from(role) as i32;
        let joined_at: Option<DateTime<Utc>> = row.try_get("joined_at").ok();

        Ok(Self {
            id: row.try_get("id")?,
            group_id: row.try_get("group_id")?,
            user_id: row.try_get("user_id")?,
            username: row.try_get("username").ok(),
            nickname: row.try_get("nickname").ok(),
            avatar_url: row.try_get("avatar_url").ok(),
            role,
            joined_at: joined_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            is_muted: false,
            mute_info: None,
            remark: "".to_string(),
        })
    }
}

// implement slqx FromRow trait
impl FromRow<'_, PgRow> for GroupInfo {
    fn from_row(row: &PgRow) -> Result<Self, Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            owner: row.try_get("owner")?,
            avatar: row.try_get("avatar")?,
            description: row.try_get("description")?,
            announcement: row.try_get("announcement")?,
            create_time: row.try_get("create_time")?,
            update_time: row.try_get("update_time")?,
        })
    }
}

