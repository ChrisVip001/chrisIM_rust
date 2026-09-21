use chrono::{DateTime, Utc};
use common::proto::friend::{DetailedFriend as ProtoDetailedFriend, FriendType,
                            Friendship as ProtoFriendship, PotentialFriend as ProtoPotentialFriend};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Friendship {
    pub id: String,
    pub user_id: String,
    pub friend_id: String,
    pub message: String,
    pub status: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reject_reason: Option<String>,
    pub friend_username: Option<String>,
    pub friend_nickname: Option<String>,
    pub friend_avatar_url: Option<String>,
    pub friend_remark: Option<String>,
}

impl Friendship {
    pub fn new(user_id: String, friend_id: String, message: String) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            friend_id,
            message,
            status: 0, // PENDING
            created_at: now,
            updated_at: now,
            reject_reason: None,
            friend_username: None,
            friend_nickname: None,
            friend_avatar_url: None,
            friend_remark: None,
        }
    }

    pub fn to_proto(&self) -> ProtoFriendship {
        ProtoFriendship {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            friend_id: self.friend_id.clone(),
            message: self.message.clone(),
            status: self.status,
            created_at: Some(prost_types::Timestamp::from(SystemTime::from(self.created_at))),
            updated_at: Some(prost_types::Timestamp::from(SystemTime::from(self.updated_at))),
            reject_reason: self.reject_reason.clone(),
            friend_username: self.friend_username.clone(),
            friend_nickname: self.friend_nickname.clone(),
            friend_avatar_url: self.friend_avatar_url.clone(),
            friend_remark: self.friend_remark.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Friend {
    pub id: String,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub friendship_created_at: DateTime<Utc>,
    pub remark: Option<String>,
}

impl Friend {
    pub fn to_proto(&self) -> common::proto::friend::Friend {
        let created_system_time = SystemTime::from(self.friendship_created_at);

        common::proto::friend::Friend {
            id: self.id.clone(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            friendship_created_at: Some(prost_types::Timestamp::from(created_system_time)),
            remark: self.remark.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FriendGroup {
    pub id: String,
    pub user_id: String,
    pub group_name: String,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub friend_count: i32,
}

impl FriendGroup {
    pub fn new(user_id: String, group_name: String, sort_order: i32) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            group_name,
            sort_order,
            created_at: now,
            updated_at: now,
            friend_count: 0,
        }
    }

    pub fn to_proto(&self) -> common::proto::friend::FriendGroup {
        common::proto::friend::FriendGroup {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            group_name: self.group_name.clone(),
            sort_order: self.sort_order,
            created_at: Some(prost_types::Timestamp::from(SystemTime::from(self.created_at))),
            updated_at: Some(prost_types::Timestamp::from(SystemTime::from(self.updated_at))),
            friend_count: self.friend_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PotentialFriend {
    pub id: String,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub friendship_status: i32,
    pub sign: Option<String>,
}

impl PotentialFriend {
    pub fn to_proto(&self) -> ProtoPotentialFriend {
        ProtoPotentialFriend {
            id: self.id.clone(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            phone: self.phone.clone(),
            friendship_status: self.friendship_status,
            sign: self.sign.clone(),
        }
    }
    
    pub fn from_tuple(
        id: String,
        username: String,
        nickname: Option<String>,
        avatar_url: Option<String>,
        phone: Option<String>,
        friendship_status: i32,
        sign: Option<String>,
        custom_id: String,
    ) -> Self {
        Self {
            id,
            username,
            nickname,
            avatar_url,
            phone,
            friendship_status,
            sign
        }
    }
}

/// 好友详细信息，包含扩展字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedFriend {
    pub id: String,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub friendship_created_at: DateTime<Utc>,
    pub remark: Option<String>,
    pub is_online: bool,
    pub is_starred: bool,
    pub is_top: bool,
    pub relation_status: i32,
    pub friend_type: i32,  // 好友类型: 0-普通好友 1-官方账号 2-系统账号
}

impl DetailedFriend {
    pub fn to_proto(&self) -> ProtoDetailedFriend {
        let created_system_time = SystemTime::from(self.friendship_created_at);

        ProtoDetailedFriend {
            id: self.id.clone(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            friendship_created_at: Some(prost_types::Timestamp::from(created_system_time)),
            remark: self.remark.clone(),
            is_online: self.is_online,
            is_starred: self.is_starred,
            is_top: self.is_top,
            relation_status: self.relation_status,
            friend_type: self.get_friend_type().into(),
        }
    }
    
    // 将内部的friend_type整数转换为proto的枚举类型
    fn get_friend_type(&self) -> FriendType {
        match self.friend_type {
            1 => FriendType::Official,
            2 => FriendType::System,
            _ => FriendType::Friend,
        }
    }
    
    // 从基本Friend转换，使用整数类型的星标和置顶状态
    pub fn from_friend(friend: Friend, is_online: bool, is_starred: i32, is_top: i32, relation_status: i32, friend_type: i32) -> Self {
        Self {
            id: friend.id.clone(),
            username: friend.username.clone(),
            nickname: friend.nickname.clone(),
            avatar_url: friend.avatar_url.clone(),
            friendship_created_at: friend.friendship_created_at,
            remark: friend.remark.clone(),
            is_online,
            is_starred: is_starred == 1,
            is_top: is_top == 1,
            relation_status,
            friend_type,
        }
    }

    // 辅助方法 - 将详细好友对象转换为proto对象
    pub fn detailed_friend_to_proto(&self) -> common::proto::friend::DetailedFriend {
        let created_system_time = SystemTime::from(self.friendship_created_at);

        let friend_type = match self.friend_type {
            0 => FriendType::Friend,
            1 => FriendType::Official,
            2 => FriendType::System,
            _ => FriendType::Friend,
        };

        common::proto::friend::DetailedFriend {
            id: self.id.clone(),
            username: self.username.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
            friendship_created_at: Some(prost_types::Timestamp::from(created_system_time)),
            remark: self.remark.clone(),
            is_online: self.is_online,
            is_starred: self.is_starred,
            is_top: self.is_top,
            relation_status: self.relation_status,
            friend_type: friend_type as i32,
        }
    }
}
