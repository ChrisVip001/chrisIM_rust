use chrono::{DateTime, Utc};
use common::proto::user;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// 用户数据库模型
#[derive(Debug, Clone, Serialize, Deserialize,FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub password: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub phone: String,
    pub address: Option<String>,
    pub head_image: Option<String>,
    pub head_image_thumb: Option<String>,
    pub sex: Option<i32>,
    pub user_stat: i32,
    pub tenant_id: String,
    pub last_login_time: Option<DateTime<Utc>>,
    pub user_idx: Option<String>,
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
    pub address: Option<String>,
    pub head_image: Option<String>,
    pub head_image_thumb: Option<String>,
    pub sex: Option<u32>,
    pub user_id: String,
}

impl From<User> for user::User {
    fn from(user: User) -> Self {
        use prost_types::Timestamp;

        Self {
            id: user.id.to_string(),
            username: user.username,
            email: user.email.unwrap_or_default(),
            nickname: user.nickname,
            avatar_url: user.avatar_url,
            created_at: Some(Timestamp {
                seconds: user.created_at.timestamp(),
                nanos: user.created_at.timestamp_subsec_nanos() as i32,
            }),
            updated_at: user.updated_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            phone: user.phone,
            address: user.address,
            head_image: user.head_image,
            head_image_thumb: user.head_image_thumb,
            sex: user.sex,
            user_stat: user.user_stat,
            tenant_id: user.tenant_id,
            last_login_time: user.last_login_time.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            user_idx: user.user_idx,
        }
    }
}

impl From<user::CreateUserRequest> for CreateUserData {
    fn from(req: user::CreateUserRequest) -> Self {
        Self {
            username: req.username,
            email: req.email,
            password: req.password,
            nickname: if req.nickname.is_empty() {
                None
            } else {
                Some(req.nickname)
            },
            avatar_url: if req.avatar_url.is_empty() {
                None
            } else {
                Some(req.avatar_url)
            },
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
            address: req.address,
            head_image: req.head_image,
            head_image_thumb: req.head_image_thumb,
            sex: req.sex.map(|x| x as u32),
            user_id: req.user_id
        }
    }
}

/// 用户注册请求数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterUserData {
    pub username: String,
    pub password: String,
    pub nickname: Option<String>,
    pub tenant_id : String,
    pub phone: String,
}

impl From<user::RegisterRequest> for RegisterUserData {
    fn from(req: user::RegisterRequest) -> Self {
        Self {
            username: req.username,
            password: req.password,
            nickname: if req.nickname.is_empty() { None } else { Some(req.nickname) },
            tenant_id: req.tenant_id,
            phone: req.phone,
        }
    }
}

/// 忘记密码请求数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetPasswordData {
    pub username: String,
    pub password: String,
    pub tenant_id : String,
    pub phone: String,
}

impl From<user::ForgetPasswordRequest> for ForgetPasswordData {
    fn from(req: user::ForgetPasswordRequest) -> Self {
        Self {
            username: req.username,
            password: req.password,
            tenant_id: req.tenant_id,
            phone: req.phone,
        }
    }
}
