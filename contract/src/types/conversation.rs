//! 会话键：全链路唯一推导来源
//!
//! 会话键同时决定三件事：Kafka 分区归属、落库时的会话序号、网关上行 worker 保序。
//! 三处必须落在同一个会话上，因此推导只在本模块存在一份实现：
//!
//! - 单聊 `d:{较小的 user_id}:{较大的 user_id}`（方向无关）
//! - 群聊 `g:{group_id}`
//!
//! 约束：本模块只依赖 proto 生成的类型，不做任何 IO；
//! 键格式是持久化数据的一部分，改动会造成历史消息错序，禁止随意更改。

use crate::proto::message::{Msg, MsgType};

/// 单聊会话 ID：两侧调用必须得到同一结果，因此按字典序排序
pub fn single_conversation_id(a: &str, b: &str) -> String {
    if a <= b {
        format!("d:{}:{}", a, b)
    } else {
        format!("d:{}:{}", b, a)
    }
}

/// 群聊会话 ID
pub fn group_conversation_id(group_id: &str) -> String {
    format!("g:{}", group_id)
}

/// 根据消息推导会话 ID
///
/// 群操作类消息（入群、退群、解散等）同样归属群会话。
pub fn conversation_id_of(msg: &Msg) -> String {
    let msg_type = MsgType::try_from(msg.msg_type).unwrap_or(MsgType::SingleMsg);
    match msg_type {
        MsgType::GroupMsg
        | MsgType::GroupInvitation
        | MsgType::GroupInviteNew
        | MsgType::GroupMemberExit
        | MsgType::GroupRemoveMember
        | MsgType::GroupDismiss
        | MsgType::GroupDismissOrExitReceived
        | MsgType::GroupInvitationReceived
        | MsgType::GroupUpdate => {
            if msg.group_id.is_empty() {
                group_conversation_id(&msg.receiver_id)
            } else {
                group_conversation_id(&msg.group_id)
            }
        }
        _ => single_conversation_id(&msg.send_id, &msg.receiver_id),
    }
}

/// 会话是单聊还是群聊
pub fn conv_type_of(msg: &Msg) -> i16 {
    let msg_type = MsgType::try_from(msg.msg_type).unwrap_or(MsgType::SingleMsg);
    match msg_type {
        MsgType::GroupMsg
        | MsgType::GroupInvitation
        | MsgType::GroupInviteNew
        | MsgType::GroupMemberExit
        | MsgType::GroupRemoveMember
        | MsgType::GroupDismiss
        | MsgType::GroupDismissOrExitReceived
        | MsgType::GroupInvitationReceived
        | MsgType::GroupUpdate => 2,
        _ => 1,
    }
}

/// 从单聊会话 ID 解析出对端用户
pub fn peer_of_single(conversation_id: &str, user_id: &str) -> Option<String> {
    let rest = conversation_id.strip_prefix("d:")?;
    let (a, b) = rest.split_once(':')?;
    if a == user_id {
        Some(b.to_string())
    } else if b == user_id {
        Some(a.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(send: &str, recv: &str) -> Msg {
        Msg {
            send_id: send.into(),
            receiver_id: recv.into(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        }
    }

    fn group(send: &str, group_id: &str) -> Msg {
        Msg {
            send_id: send.into(),
            group_id: group_id.into(),
            msg_type: MsgType::GroupMsg as i32,
            ..Default::default()
        }
    }

    #[test]
    fn single_is_order_independent() {
        assert_eq!(single_conversation_id("u1", "u2"), "d:u1:u2");
        assert_eq!(single_conversation_id("u2", "u1"), "d:u1:u2");
        assert_eq!(conversation_id_of(&single("u1", "u2")), conversation_id_of(&single("u2", "u1")));
    }

    #[test]
    fn group_is_sender_independent() {
        assert_eq!(conversation_id_of(&group("u1", "g1")), "g:g1");
        assert_eq!(conversation_id_of(&group("u2", "g1")), "g:g1");
    }

    #[test]
    fn group_falls_back_to_receiver() {
        let mut msg = group("u1", "");
        msg.receiver_id = "g9".into();
        assert_eq!(conversation_id_of(&msg), "g:g9");
        assert_eq!(conv_type_of(&msg), 2);
    }

    #[test]
    fn peer_resolution() {
        assert_eq!(peer_of_single("d:u1:u2", "u1"), Some("u2".into()));
        assert_eq!(peer_of_single("d:u1:u2", "u3"), None);
        assert_eq!(peer_of_single("g:g1", "u1"), None);
    }
}
