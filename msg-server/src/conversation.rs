//! 会话列表的查询范围与聚合逻辑
//!
//! 这部分原本混在 1400 行的 `productor` 里，但它和 gRPC 无关：
//! 输入是序列号水位和消息集合，输出是会话列表。抽出来之后
//! 可以脱离服务实例直接做单元测试。

use std::collections::HashMap;

use cache::Cache;
use contract::error::Error;
use contract::proto::message::{Conversation, GetConversationsRequest, Msg, MsgType};
use contract::types::msg::MsgType2;
use msg_storage::message::MsgRecBoxRepo;

/// 离线全量模式最多回溯的消息条数
const OFFLINE_RECENT_COUNT: i64 = 5000;
/// 正常模式（打开应用首屏）最多回溯的消息条数
const NORMAL_RECENT_COUNT: i64 = 2000;

/// 查询模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryMode {
    /// 增量同步：获取指定序列号之后的新消息
    IncrementalSync,
    /// 全量离线：获取所有离线消息
    FullOffline,
    /// 正常模式：获取最近N条消息
    Normal,
}

/// 查询范围
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueryRange {
    pub mode: QueryMode,
    pub rec_start: i64,
    pub rec_end: i64,
    pub send_start: i64,
    pub send_end: i64,
}

impl QueryRange {
    /// 收发两个方向都没有新消息
    pub(crate) fn is_empty(&self) -> bool {
        self.rec_start > self.rec_end && self.send_start > self.send_end
    }
}

/// 根据客户端上报的水位确定查询模式和区间
///
/// - 客户端带了 `since_seq`/`since_send_seq` 且开启同步：增量拉取 `since+1 ..= 当前`
/// - 开启同步但没带水位，或两个水位都是 0：按离线处理，回溯最近 5000 条
/// - 其余情况：按首屏处理，回溯最近 2000 条
///
/// 上界一律用服务端当前水位，避免客户端上报的越界序列号拉出空结果。
pub(crate) fn determine_query_range(
    req: &GetConversationsRequest,
    rec_seq: i64,
    send_seq: i64,
) -> QueryRange {
    let sync_mode = req.sync_mode.unwrap_or(false);
    let since_seq = req.since_seq.unwrap_or(0);
    let since_send_seq = req.since_send_seq.unwrap_or(0);

    let mode = if sync_mode && (since_seq > 0 || since_send_seq > 0) {
        QueryMode::IncrementalSync
    } else if sync_mode || (since_seq == 0 && since_send_seq == 0) {
        QueryMode::FullOffline
    } else {
        QueryMode::Normal
    };

    match mode {
        QueryMode::IncrementalSync => QueryRange {
            mode,
            rec_start: since_seq + 1,
            rec_end: rec_seq,
            send_start: since_send_seq + 1,
            send_end: send_seq,
        },
        QueryMode::FullOffline => QueryRange {
            mode,
            rec_start: std::cmp::max(1, rec_seq - OFFLINE_RECENT_COUNT),
            rec_end: rec_seq,
            send_start: std::cmp::max(1, send_seq - OFFLINE_RECENT_COUNT),
            send_end: send_seq,
        },
        QueryMode::Normal => QueryRange {
            mode,
            rec_start: std::cmp::max(1, rec_seq - NORMAL_RECENT_COUNT),
            rec_end: rec_seq,
            send_start: std::cmp::max(1, send_seq - NORMAL_RECENT_COUNT),
            send_end: send_seq,
        },
    }
}

/// 按查询范围拉取消息
pub(crate) async fn fetch_messages(
    repo: &dyn MsgRecBoxRepo,
    user_id: &str,
    range: &QueryRange,
) -> Result<Vec<Msg>, Error> {
    repo.get_msgs(
        user_id,
        range.send_start,
        range.send_end,
        range.rec_start,
        range.rec_end,
    )
    .await
}

/// 把消息聚合成会话列表
pub(crate) async fn build_conversations(
    cache: &dyn Cache,
    user_id: &str,
    messages: Vec<Msg>,
) -> Vec<Conversation> {
    if messages.is_empty() {
        return Vec::new();
    }

    let conversations_map = group_messages_by_conversation(user_id, messages);

    let mut conversations = Vec::with_capacity(conversations_map.len());
    for (conversation_id, mut msgs) in conversations_map {
        if msgs.is_empty() {
            continue;
        }

        // 按发送时间排序（最新的在后）
        msgs.sort_by_key(|msg| msg.send_time);
        conversations.push(create_conversation(cache, conversation_id, msgs, user_id).await);
    }

    // 按最后活跃时间排序（最新的在后）
    conversations.sort_by_key(|conversation| conversation.last_active_time);
    conversations
}

/// 按会话ID分组消息
pub(crate) fn group_messages_by_conversation(
    user_id: &str,
    messages: Vec<Msg>,
) -> HashMap<String, Vec<Msg>> {
    let mut conversations: HashMap<String, Vec<Msg>> = HashMap::new();

    for msg in messages {
        let conversation_id = extract_conversation_id(&msg, user_id);
        if !conversation_id.is_empty() {
            conversations
                .entry(conversation_id)
                .or_default()
                .push(msg);
        }
    }

    conversations
}

/// 提取消息所属的会话ID
///
/// 群聊/群操作类消息的会话ID是群组ID；
/// 单聊消息的会话ID是「对方」的用户ID —— 自己发的取接收者，收到的取发送者。
pub(crate) fn extract_conversation_id(msg: &Msg, user_id: &str) -> String {
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
        | MsgType::GroupUpdate => msg.group_id.clone(),
        _ => {
            if msg.send_id == user_id {
                msg.receiver_id.clone()
            } else {
                msg.send_id.clone()
            }
        }
    }
}

/// 由一组同会话消息构造会话对象，并刷新缓存中的未读数
async fn create_conversation(
    cache: &dyn Cache,
    conversation_id: String,
    msgs: Vec<Msg>,
    user_id: &str,
) -> Conversation {
    let mt = MsgType::try_from(msgs[0].msg_type).unwrap_or(MsgType::SingleMsg);
    let mt2 = MsgType2::from(mt);

    // 未读只统计：我收到的、未读的、未撤回的单聊/群聊消息
    let unread_count = msgs
        .iter()
        .filter(|msg| {
            !msg.is_read
                && msg.receiver_id == user_id
                && !msg.is_revoked
                && (msg.msg_type == MsgType::SingleMsg as i32
                    || msg.msg_type == MsgType::GroupMsg as i32)
        })
        .count() as i32;

    let _ = cache
        .unread_count_set(user_id, &conversation_id, &unread_count)
        .await;

    let last_active_time = msgs[msgs.len() - 1].send_time;

    Conversation {
        conversation_id,
        conversation_type: MsgType2::mt2_str(mt2),
        recent_messages: msgs,
        unread_count,
        last_active_time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(sync_mode: Option<bool>, since_seq: Option<i64>, since_send_seq: Option<i64>) -> GetConversationsRequest {
        GetConversationsRequest {
            user_id: "u1".into(),
            sync_mode,
            since_seq,
            since_send_seq,
        }
    }

    fn msg(send_id: &str, receiver_id: &str, msg_type: MsgType, send_time: i64) -> Msg {
        Msg {
            send_id: send_id.into(),
            receiver_id: receiver_id.into(),
            msg_type: msg_type as i32,
            send_time,
            ..Default::default()
        }
    }

    #[test]
    fn incremental_sync_starts_right_after_client_watermark() {
        let range = determine_query_range(&req(Some(true), Some(10), Some(3)), 20, 8);

        assert_eq!(range.mode, QueryMode::IncrementalSync);
        assert_eq!((range.rec_start, range.rec_end), (11, 20));
        assert_eq!((range.send_start, range.send_end), (4, 8));
        assert!(!range.is_empty());
    }

    #[test]
    fn incremental_sync_with_no_new_messages_is_empty() {
        // 客户端水位已经追平服务端
        let range = determine_query_range(&req(Some(true), Some(20), Some(8)), 20, 8);

        assert_eq!(range.mode, QueryMode::IncrementalSync);
        assert!(range.is_empty(), "没有新消息时不应触发任何查询");
    }

    #[test]
    fn sync_mode_without_watermark_falls_back_to_full_offline() {
        let range = determine_query_range(&req(Some(true), None, None), 20, 8);

        assert_eq!(range.mode, QueryMode::FullOffline);
        assert_eq!((range.rec_start, range.rec_end), (1, 20));
        assert_eq!((range.send_start, range.send_end), (1, 8));
    }

    #[test]
    fn offline_range_is_capped_by_window_size() {
        let rec_seq = OFFLINE_RECENT_COUNT + 1234;
        let send_seq = OFFLINE_RECENT_COUNT + 10;
        let range = determine_query_range(&req(Some(true), None, None), rec_seq, send_seq);

        // 下界被截断到窗口大小，避免一次性把全量历史拉进内存
        assert_eq!(range.rec_start, rec_seq - OFFLINE_RECENT_COUNT);
        assert_eq!(range.send_start, send_seq - OFFLINE_RECENT_COUNT);
        assert_eq!(range.rec_end, rec_seq);
        assert_eq!(range.send_end, send_seq);
    }

    #[test]
    fn normal_mode_uses_smaller_window() {
        // 注意：两个水位都为 0 时走的是离线模式；只有客户端上报了水位
        // 但没有开启同步时才是首屏的 Normal 模式
        let range = determine_query_range(&req(None, Some(0), Some(1)), 5000, 5000);

        assert_eq!(range.mode, QueryMode::Normal);
        assert_eq!(range.rec_start, 5000 - NORMAL_RECENT_COUNT);
        assert_eq!(range.send_start, 5000 - NORMAL_RECENT_COUNT);
    }

    #[test]
    fn small_watermark_never_goes_below_one() {
        let range = determine_query_range(&req(None, None, None), 3, 0);

        assert_eq!(range.rec_start, 1, "序列号从 1 开始，下界不能为负");
        assert_eq!(range.send_start, 1);
    }

    #[test]
    fn single_chat_conversation_id_is_the_peer() {
        let sent = msg("u1", "u2", MsgType::SingleMsg, 1);
        let received = msg("u2", "u1", MsgType::SingleMsg, 1);

        assert_eq!(extract_conversation_id(&sent, "u1"), "u2");
        assert_eq!(extract_conversation_id(&received, "u1"), "u2");
    }

    #[test]
    fn group_conversation_id_is_the_group() {
        let mut group_msg = msg("u1", "g1", MsgType::GroupMsg, 1);
        group_msg.group_id = "g1".into();

        assert_eq!(extract_conversation_id(&group_msg, "u1"), "g1");
    }

    #[test]
    fn messages_are_grouped_per_conversation() {
        let messages = vec![
            msg("u1", "u2", MsgType::SingleMsg, 1),
            msg("u2", "u1", MsgType::SingleMsg, 2),
            {
                let mut m = msg("u1", "g1", MsgType::GroupMsg, 3);
                m.group_id = "g1".into();
                m
            },
            {
                let mut m = msg("u3", "g1", MsgType::GroupMsg, 4);
                m.group_id = "g1".into();
                m
            },
        ];

        let grouped = group_messages_by_conversation("u1", messages);

        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["u2"].len(), 2);
        assert_eq!(grouped["g1"].len(), 2);
    }

    #[test]
    fn messages_without_conversation_id_are_dropped() {
        // 既没有 group_id 又恰好是自己发给自己，会话归属不明确，直接丢弃
        let messages = vec![msg("u1", "", MsgType::GroupMsg, 1)];
        let grouped = group_messages_by_conversation("u1", messages);

        assert!(grouped.is_empty());
    }
}
