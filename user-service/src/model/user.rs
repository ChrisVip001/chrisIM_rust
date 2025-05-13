use chrono::{DateTime, Utc};
use common::proto::user;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 用户数据库模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub password: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 创建用户请求数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserData {
    pub username: String,
    pub email: String,
    pub password: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

/// 更新用户请求数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserData {
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub password: Option<String>,
}

impl From<User> for user::User {
    fn from(user: User) -> Self {
        use prost_types::Timestamp;
        
        Self {
            id: user.id.to_string(),
            username: user.username,
            email: user.email,
            nickname: user.nickname,
            avatar_url: user.avatar_url,
            created_at: Some(Timestamp {
                seconds: user.created_at.timestamp(),
                nanos: user.created_at.timestamp_subsec_nanos() as i32,
            }),
            updated_at: Some(Timestamp {
                seconds: user.updated_at.timestamp(),
                nanos: user.updated_at.timestamp_subsec_nanos() as i32,
            }),
        }
    }
}

impl From<user::CreateUserRequest> for CreateUserData {
    fn from(req: user::CreateUserRequest) -> Self {
        Self {
            username: req.username,
            email: req.email,
            password: req.password,
            nickname: if req.nickname.is_empty() { None } else { Some(req.nickname) },
            avatar_url: if req.avatar_url.is_empty() { None } else { Some(req.avatar_url) },
        }
    }
}

impl From<user::UpdateUserRequest> for UpdateUserData {
    fn from(req: user::UpdateUserRequest) -> Self {
        Self {
            email: req.email,
            nickname: req.nickname,
            avatar_url: req.avatar_url,
            password: req.password,
        }
    }
} 