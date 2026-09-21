use crate::proto::message::{GetDbMessagesRequest, GroupMemSeq, Msg, MsgResponse, MsgType, SaveGroupMsgRequest, SaveMessageRequest, SendMsgRequest, UserAndGroupId};
use crate::Error;
use mongodb::bson::Document;
use tonic::Status;

impl From<Status> for MsgResponse {
    fn from(status: Status) -> Self {
        MsgResponse {
            local_id: String::new(),
            server_id: String::new(),
            send_time: 0,
            err: status.message().to_string(),
        }
    }
}

/// maybe there is the performance issue
impl TryFrom<Document> for Msg {
    type Error = Error;

    fn try_from(value: Document) -> Result<Self, Self::Error> {
        Ok(Self {
            local_id: value.get_str("local_id").unwrap_or_default().to_string(),
            server_id: value.get_str("server_id").unwrap_or_default().to_string(),
            create_time: value.get_i64("create_time").unwrap_or_default(),
            send_time: value.get_i64("send_time").unwrap_or_default(),
            content_type: value.get_i32("content_type").unwrap_or_default(),
            content: value
                .get_binary_generic("content")
                .map_or(vec![], |v| v.to_vec()),
            send_id: value.get_str("send_id").unwrap_or_default().to_string(),
            receiver_id: value.get_str("receiver_id").unwrap_or_default().to_string(),
            seq: value.get_i64("seq").unwrap_or_default(),
            send_seq: value.get_i64("send_seq").unwrap_or_default(),
            msg_type: value.get_i32("msg_type").unwrap_or_default(),
            is_read: value.get_bool("is_read").unwrap_or_default(),
            group_id: value.get_str("group_id").unwrap_or_default().to_string(),
            platform: value.get_i32("platform").unwrap_or_default(),
            avatar: value.get_str("avatar").unwrap_or_default().to_string(),
            nickname: value.get_str("nickname").unwrap_or_default().to_string(),
            related_msg_id: value
                .get_str("related_msg_id")
                .ok()
                .filter(|s| !s.is_empty())
                .map(String::from),
            is_revoked: value.get_bool("is_revoked").unwrap_or_default(),
            revoke_time: value.get_i64("revoke_time").unwrap_or_default(),
            revoked_by: value.get_str("revoked_by").unwrap_or_default().to_string(),
        })
    }
}

impl SendMsgRequest {
    pub fn new_with_friend_del(send_id: String, receiver_id: String) -> Self {
        Self {
            message: Some(Msg {
                send_id,
                receiver_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                msg_type: MsgType::FriendDelete as i32,
                ..Default::default()
            }),
        }
    }

    pub fn new_with_friend_ship_req(
        send_id: String,
        receiver_id: String,
        fs: Vec<u8>,
        send_seq: i64,
    ) -> Self {
        Self {
            message: Some(Msg {
                send_seq,
                send_id,
                receiver_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                content: fs,
                msg_type: MsgType::FriendApplyReq as i32,
                ..Default::default()
            }),
        }
    }

    pub fn new_with_friend_ship_resp(receiver_id: String, fs: Vec<u8>, send_seq: i64) -> Self {
        Self {
            message: Some(Msg {
                send_seq,
                receiver_id,
                content: fs,
                msg_type: MsgType::FriendApplyResp as i32,
                send_time: chrono::Utc::now().timestamp_millis(),
                ..Default::default()
            }),
        }
    }

    /// when dismiss group, send id is the owner id,
    /// when member exit group, send id is the member id
    pub fn new_with_group_operation(
        send_id: String,
        receiver_id: String,
        msg_type: MsgType,
        send_seq: i64,
    ) -> Self {
        Self {
            message: Some(Msg {
                send_id,
                group_id: receiver_id.clone(),
                receiver_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                msg_type: msg_type as i32,
                send_seq,
                ..Default::default()
            }),
        }
    }

    pub fn new_with_group_invitation(
        send_id: String,
        receiver_id: String,
        send_seq: i64,
        invitation: Vec<u8>,
    ) -> Self {
        Self {
            message: Some(Msg {
                send_id,
                group_id: receiver_id.clone(),
                receiver_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                msg_type: MsgType::GroupInvitation as i32,
                content: invitation,
                send_seq,
                ..Default::default()
            }),
        }
    }

    pub fn new_with_group_invite_new(
        send_id: String,
        receiver_id: String,
        send_seq: i64,
        invitation: Vec<u8>,
    ) -> Self {
        Self {
            message: Some(Msg {
                send_id,
                group_id: receiver_id.clone(),
                receiver_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                msg_type: MsgType::GroupInviteNew as i32,
                content: invitation,
                send_seq,
                ..Default::default()
            }),
        }
    }

    pub fn new_with_group_remove_mem(
        send_id: String,
        group_id: String,
        send_seq: i64,
        invitation: Vec<u8>,
    ) -> Self {
        Self {
            message: Some(Msg {
                send_id,
                receiver_id: group_id.clone(),
                group_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                msg_type: MsgType::GroupRemoveMember as i32,
                content: invitation,
                send_seq,
                ..Default::default()
            }),
        }
    }

    pub fn new_with_group_update(
        send_id: String,
        receiver_id: String,
        send_seq: i64,
        msg: Vec<u8>,
    ) -> Self {
        Self {
            message: Some(Msg {
                send_id,
                group_id: receiver_id.clone(),
                receiver_id,
                send_time: chrono::Utc::now().timestamp_millis(),
                msg_type: MsgType::GroupUpdate as i32,
                content: msg,
                send_seq,
                ..Default::default()
            }),
        }
    }
}

impl UserAndGroupId {
    pub fn new(user_id: String, group_id: String) -> Self {
        Self { user_id, group_id }
    }
}


impl GroupMemSeq {
    pub fn new(mem_id: String, cur_seq: i64, max_seq: i64, need_update: bool) -> Self {
        Self {
            mem_id,
            cur_seq,
            max_seq,
            need_update,
        }
    }
}

impl GetDbMessagesRequest {
    pub fn validate(&self) -> Result<(), Error> {
        if self.user_id.is_empty() {
            return Err(Error::BadRequest("user_id is empty".to_string()));
        }
        if self.conversation_id.is_empty() {
            return Err(Error::BadRequest("conversation_id is empty".to_string()));
        }
        if self.seq_start < 0 {
            return Err(Error::BadRequest("seq_start is invalid".to_string()));
        }
        if self.seq_end < 0 {
            return Err(Error::BadRequest("seq_end is invalid".to_string()));
        }
        if self.seq_end > 0 && self.seq_end < self.seq_start {
            return Err(Error::BadRequest("seq_start is greater than seq_end".to_string()));
        }
        if self.send_seq_start < 0 {
            return Err(Error::BadRequest("send_seq_start is invalid".to_string()));
        }
        if self.send_seq_end < 0 {
            return Err(Error::BadRequest("send_seq_end is invalid".to_string()));
        }
        if self.send_seq_end > 0 && self.send_seq_end < self.send_seq_start {
            return Err(Error::BadRequest("send_seq_start is greater than send_seq_end".to_string()));
        }
        Ok(())
    }
}

impl SaveMessageRequest {
    pub fn new(msg: Msg, need_to_history: bool) -> Self {
        Self {
            message: Some(msg),
            need_to_history,
        }
    }
}

impl SaveGroupMsgRequest {
    pub fn new(msg: Msg, need_to_history: bool, members: Vec<GroupMemSeq>) -> Self {
        Self {
            message: Some(msg),
            need_to_history,
            members,
        }
    }
}

/// 消息类型的简化分类枚举
///
/// 为了简化消息处理逻辑，将复杂的消息类型归类为两种基本类型：
/// - 单聊消息：点对点的私人消息
/// - 群聊消息：一对多的群组消息
///
/// 这种分类有助于统一处理流程，避免代码重复
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum MsgType2 {
    /// 单聊消息类型
    /// 包括普通文本、图片、语音、视频等私人消息
    Friend,

    /// 群聊消息类型  
    /// 包括群聊文本、群公告、群成员变更等群组消息
    Group,

    /// 系统消息类型
    /// 包括系统通知、用户状态变更，心跳等
    System,
}

/// 简化消息类型枚举
impl From<MsgType> for MsgType2 {
    fn from(mt: MsgType) -> Self {
        match mt {
            // 单聊消息类型，需要增加序列号
            MsgType::SingleMsg
            | MsgType::SingleCallInviteNotAnswer
            | MsgType::SingleCallInviteCancel
            | MsgType::Hangup
            | MsgType::ConnectSingleCall
            | MsgType::RejectSingleCall
            | MsgType::FriendApplyReq
            | MsgType::FriendApplyResp
            | MsgType::FriendDelete
            | MsgType::FriendBlack
            | MsgType::SingleCallInvite
            | MsgType::AgreeSingleCall
            | MsgType::SingleCallOffer
            | MsgType::Candidate => {
                // 单聊消息，需要增加序列号
                MsgType2::Friend
            }
            // 群组操作消息类型
            MsgType::GroupMsg
            | MsgType::GroupDismissOrExitReceived
            | MsgType::GroupInvitationReceived
            | MsgType::GroupInvitation
            | MsgType::GroupInviteNew
            | MsgType::GroupMemberExit
            | MsgType::GroupRemoveMember
            | MsgType::GroupDismiss
            | MsgType::GroupUpdate => {
                MsgType2::Group
            }
            MsgType::Read
            | MsgType::MsgRecResp
            | MsgType::Notification
            | MsgType::Service
            | MsgType::Heartbeat => {
                MsgType2::System
            }
            _ => {
                MsgType2::Friend
            }
        }
    }
}

impl MsgType2 {

    /// 根据消息类型进行分类，确定处理策略
    ///
    /// 分析消息类型并返回处理策略，包括：
    /// - 消息归类（单聊/群聊）
    /// - 是否需要分配序列号
    /// - 是否需要存储历史记录
    ///
    /// # 参数
    /// * `msg_type` - 原始消息类型枚举
    ///
    /// # 返回值
    /// 返回元组: (简化消息类型, 是否需要序列号, 是否需要历史存储)
    pub fn classify_msg_type(mt: MsgType) -> (MsgType2, bool, bool) {
        let msg_type;
        let mut need_increase_seq = false;
        let mut need_history = true;

        match mt {
            // 单聊消息类型，需要增加序列号
            MsgType::SingleMsg
            | MsgType::SingleCallInviteNotAnswer
            | MsgType::SingleCallInviteCancel
            | MsgType::Hangup
            | MsgType::ConnectSingleCall
            | MsgType::RejectSingleCall
            | MsgType::FriendApplyReq
            | MsgType::FriendApplyResp
            | MsgType::FriendDelete => {
                // 单聊消息，需要增加序列号
                msg_type = MsgType2::Friend;
                need_increase_seq = true;
            }
            // 群聊消息类型，序列号处理方式特殊
            MsgType::GroupMsg => {
                // 群聊消息，需要增加每个成员的序列号
                // 但不是在这里处理，而是在handle_group_seq中处理
                msg_type = MsgType2::Group;
            }
            // 群组操作消息类型
            MsgType::GroupInvitation
            | MsgType::GroupInviteNew
            | MsgType::GroupMemberExit
            | MsgType::GroupRemoveMember
            | MsgType::GroupDismiss
            | MsgType::GroupUpdate => {
                // 群组消息，需要增加序列号
                msg_type = MsgType2::Group;
                need_history = false;
            }
            // 单聊通话数据交换和其他不需要增加序列号的消息
            MsgType::GroupDismissOrExitReceived
            | MsgType::GroupInvitationReceived
            | MsgType::FriendBlack
            | MsgType::SingleCallInvite
            | MsgType::AgreeSingleCall
            | MsgType::SingleCallOffer
            | MsgType::Candidate => {
                msg_type = MsgType2::Friend;
                need_history = false;
            }
            MsgType::Heartbeat
            | MsgType::Read
            | MsgType::MsgRecResp
            | MsgType::Notification
            | MsgType::Service
            | _ => {
                // 其他消息类型，不需要增加序列号
                msg_type = MsgType2::System;
                need_history = false;
            }
        }

        (msg_type, need_increase_seq, need_history)
    }
    
    /// 将MsgType2转换为字符串
    pub fn mt2_str(mt: MsgType2) -> String {
        match mt {
            MsgType2::Friend => String::from("friend"),
            MsgType2::Group => String::from("group"),
            MsgType2::System => String::from("system"),
        }
    }
}
