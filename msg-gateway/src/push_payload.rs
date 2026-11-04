//! 下行推送报文的数据结构
//!
//! 网关向客户端推送的 JSON 结构，字段与 `GetConversationsResponse` 保持一致，
//! 客户端拉取会话列表与接收实时推送可以复用同一套解析逻辑。
//!
//! 相比直接修改 `serde_json::Value` 再塞 `is_self`，这里用强类型 DTO 表达：
//! - 字段拼写由编译器检查，改名不会静默失效；
//! - `is_self` 是 `PushMessage` 的普通字段，不再依赖多层 `if let` 定位；
//! - 序列化成本不随字段嵌套深度变化。

use serde::Serialize;

use contract::proto::message::Msg;

/// 下行推送的顶层报文
#[derive(Debug, Serialize)]
pub(crate) struct PushPayload {
    /// 会话列表，实时推送时固定只有一个元素
    pub conversations: Vec<PushConversation>,
    /// 会话总数
    pub total: i32,
    /// 当前接收序列号水位
    pub seq_max: i64,
    /// 当前发送序列号水位
    pub send_seq_max: i64,
}

/// 单个会话
#[derive(Debug, Serialize)]
pub(crate) struct PushConversation {
    pub conversation_id: String,
    pub conversation_type: String,
    pub recent_messages: Vec<PushMessage>,
    pub unread_count: i32,
    pub last_active_time: i64,
}

/// 带方向标记的单条消息
#[derive(Debug, Serialize)]
pub(crate) struct PushMessage {
    /// 原样展开消息的全部字段，保持与历史协议的线格式兼容
    #[serde(flatten)]
    pub msg: Msg,
    /// true 表示这条消息是发送者自己在其他端产生的副本
    pub is_self: bool,
}

impl PushPayload {
    /// 构造「一个会话 + 一条消息」的推送报文
    pub(crate) fn single(
        msg: Msg,
        conversation_id: String,
        conversation_type: String,
        unread_count: i32,
        is_self: bool,
    ) -> Self {
        let seq_max = msg.seq;
        let send_seq_max = msg.send_seq;
        let last_active_time = msg.send_time;

        Self {
            conversations: vec![PushConversation {
                conversation_id,
                conversation_type,
                recent_messages: vec![PushMessage { msg, is_self }],
                unread_count,
                last_active_time,
            }],
            total: 1,
            seq_max,
            send_seq_max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_msg() -> Msg {
        Msg {
            send_id: "u1".into(),
            receiver_id: "u2".into(),
            server_id: "s1".into(),
            seq: 7,
            send_seq: 3,
            send_time: 1_700_000_000_000,
            msg_type: 1,
            ..Default::default()
        }
    }

    #[test]
    fn payload_keeps_wire_compatibility() {
        let payload = PushPayload::single(sample_msg(), "u2".into(), "friend".into(), 5, false);
        let value = serde_json::to_value(&payload).unwrap();

        // 顶层字段名必须与拉取会话列表的响应一致
        assert_eq!(value["total"], 1);
        assert_eq!(value["seq_max"], 7);
        assert_eq!(value["send_seq_max"], 3);

        let conversation = &value["conversations"][0];
        assert_eq!(conversation["conversation_id"], "u2");
        assert_eq!(conversation["conversation_type"], "friend");
        assert_eq!(conversation["unread_count"], 5);
        assert_eq!(conversation["last_active_time"], 1_700_000_000_000i64);
    }

    #[test]
    fn is_self_is_flat_on_message() {
        let payload = PushPayload::single(sample_msg(), "u2".into(), "friend".into(), 0, true);
        let value = serde_json::to_value(&payload).unwrap();
        let message = &value["conversations"][0]["recent_messages"][0];

        // is_self 与消息字段处于同一层，而不是嵌套在某个 msg 对象里
        assert_eq!(message["is_self"], true);
        assert_eq!(message["send_id"], "u1");
        assert_eq!(message["server_id"], "s1");
        assert!(message.get("msg").is_none());
    }

    #[test]
    fn self_copy_does_not_carry_unread_count() {
        let payload = PushPayload::single(sample_msg(), "u2".into(), "friend".into(), 0, true);
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["conversations"][0]["unread_count"], 0);
    }
}
