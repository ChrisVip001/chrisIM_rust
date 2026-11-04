//! 读扩散存储抽象
//!
//! 一条消息只写一次，写入成本与群规模无关。对应
//! `docs/MESSAGES_STORAGE_DDL.sql` 的四张表：
//!
//! | trait | 表 |
//! |---|---|
//! | [`ConversationRepo`] | `conversation` |
//! | [`MsgMetaRepo`] | `msg_meta` + `msg_content` |
//! | [`UserConversationRepo`] | `user_conversation` |
//!
//! 与旧的 `MsgStoreRepo` / `MsgRecBoxRepo` 的根本差别：没有 user 维度的消息副本。
//! 拉取按 `(conversation_id, seq)` 主键扫描，已读是游标而非行级标记。

use std::fmt::Debug;

use async_trait::async_trait;
use contract::error::Error;
use contract::proto::message::{Msg, MsgType};

/// 会话状态
#[derive(Debug, Clone, Default)]
pub struct ConversationState {
    pub conversation_id: String,
    /// 1=单聊，2=群聊
    pub conv_type: i16,
    /// 号段水位：Redis 号段预留到的上界（会跳号）
    pub last_seq: i64,
    /// 实际最后一条消息的序号，未读数基准
    pub last_msg_seq: i64,
    /// 最后一条消息时间（毫秒）
    pub last_msg_time: i64,
}

/// 用户在某会话中的状态
#[derive(Debug, Clone, Default)]
pub struct UserConversationState {
    pub conversation_id: String,
    pub last_read_seq: i64,
    pub is_pinned: bool,
    pub is_muted: bool,
}

/// 待写入的消息
#[derive(Debug, Clone)]
pub struct MsgEntry {
    pub conversation_id: String,
    pub seq: i64,
    pub msg: Msg,
}

impl MsgEntry {
    pub fn new(conversation_id: impl Into<String>, seq: i64, msg: Msg) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            seq,
            msg,
        }
    }

    /// 正文是否应当内联进 `msg_meta`
    pub fn should_inline(&self, max_bytes: usize) -> bool {
        self.msg.content.len() <= max_bytes
    }
}

/// 会话序号与元信息仓库
#[async_trait]
pub trait ConversationRepo: Sync + Send + Debug {
    /// 分配会话下一个序号
    ///
    /// 当前实现走 PostgreSQL 原子自增（`UPDATE ... RETURNING`）。
    /// 热群会产生行锁争用，后续应换成 Redis 号段 + [`Self::flush_seq`] 回写。
    async fn next_seq(&self, conversation_id: &str, conv_type: i16) -> Result<i64, Error>;

    /// 确保会话存在，返回当前状态
    async fn ensure(&self, conversation_id: &str, conv_type: i16) -> Result<ConversationState, Error>;

    /// 读取会话状态
    async fn get(&self, conversation_id: &str) -> Result<Option<ConversationState>, Error>;

    /// 回写序号水位（号段用尽时由 Redis 号段持有者调用）
    async fn flush_seq(&self, conversation_id: &str, last_seq: i64) -> Result<(), Error>;

    /// 更新实际消息水位与最后消息时间
    ///
    /// 只影响 `last_msg_seq` / `last_msg_time`，不触碰号段水位 `last_seq`，
    /// 否则号段跳号会被误当成「有这么多条未读」。
    async fn touch(
        &self,
        conversation_id: &str,
        last_msg_seq: i64,
        last_msg_time: i64,
    ) -> Result<(), Error>;

    /// 删除会话（群解散时使用）
    async fn delete(&self, conversation_id: &str) -> Result<(), Error>;
}

/// 消息元数据与正文仓库
#[async_trait]
pub trait MsgMetaRepo: Sync + Send + Debug {
    /// 写入一条消息
    ///
    /// 幂等：`(conversation_id, server_id)` 冲突时忽略，返回 `false`。
    /// 正文超过内联阈值时同时写 `msg_content`，两步在同一事务内。
    async fn insert(&self, entry: &MsgEntry, inline_max_bytes: usize) -> Result<bool, Error>;

    /// 批量写入（同一事务），返回实际写入条数
    async fn insert_batch(&self, entries: &[MsgEntry], inline_max_bytes: usize)
        -> Result<u64, Error>;

    /// 按会话增量拉取
    ///
    /// 返回按 `seq` 升序的消息，正文已补齐（内联或回表）。
    async fn fetch_after(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<Msg>, Error>;

    /// 按会话拉取某个区间（客户端补洞用）
    async fn fetch_range(
        &self,
        conversation_id: &str,
        start_seq: i64,
        end_seq: i64,
    ) -> Result<Vec<Msg>, Error>;

    /// 单条查询（幂等闸门用）
    async fn get(&self, conversation_id: &str, server_id: &str) -> Result<Option<Msg>, Error>;

    /// 撤回
    async fn revoke(
        &self,
        conversation_id: &str,
        seq: i64,
        revoked_by: &str,
        revoke_time: i64,
    ) -> Result<(), Error>;

    /// 按消息ID删除（用户删除自己的可见消息时，读扩散下不物理删除，
    /// 由 `user_conversation` 记录隐藏位点）
    async fn delete_by_server_id(
        &self,
        conversation_id: &str,
        server_ids: &[String],
    ) -> Result<u64, Error>;

    /// 删除整个会话的消息
    async fn delete_conversation(&self, conversation_id: &str) -> Result<u64, Error>;
}

/// 用户会话游标仓库
#[async_trait]
pub trait UserConversationRepo: Sync + Send + Debug {
    /// 登记用户与会话的关系（发送消息或入群时调用）
    async fn ensure_member(&self, user_id: &str, conversation_id: &str) -> Result<(), Error>;

    /// 批量登记成员（群发消息时一次性写入）
    async fn ensure_members(&self, user_ids: &[String], conversation_id: &str)
        -> Result<u64, Error>;

    /// 标记已读到指定序号（只增不减）
    async fn mark_read(
        &self,
        user_id: &str,
        conversation_id: &str,
        read_seq: i64,
    ) -> Result<(), Error>;

    /// 读取已读游标
    async fn get_cursor(&self, user_id: &str, conversation_id: &str) -> Result<i64, Error>;

    /// 列出用户的会话（按入参顺序无关，排序由上层结合 conversation 决定）
    async fn list(&self, user_id: &str, limit: i64) -> Result<Vec<UserConversationState>, Error>;

    /// 未读数 = 会话水位 - 已读游标
    async fn unread(&self, user_id: &str, conversation_id: &str) -> Result<i64, Error>;
}

// ---------------------------------------------------------------------------
// 会话 ID 生成
// ---------------------------------------------------------------------------

/// 单聊会话 ID：`d:{较小的 user_id}:{较大的 user_id}`
///
/// 两侧调用必须得到同一结果，因此按字典序排序而不是「发送者/接收者」。
pub fn single_conversation_id(a: &str, b: &str) -> String {
    if a <= b {
        format!("d:{}:{}", a, b)
    } else {
        format!("d:{}:{}", b, a)
    }
}

/// 群聊会话 ID：`g:{group_id}`
pub fn group_conversation_id(group_id: &str) -> String {
    format!("g:{}", group_id)
}

/// 根据消息类型推导会话 ID
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

    #[test]
    fn single_conversation_id_is_order_independent() {
        assert_eq!(single_conversation_id("u1", "u2"), "d:u1:u2");
        assert_eq!(single_conversation_id("u2", "u1"), "d:u1:u2");
    }

    #[test]
    fn group_conversation_id_has_prefix() {
        assert_eq!(group_conversation_id("g1"), "g:g1");
    }

    #[test]
    fn conversation_id_of_single_chat_is_stable_for_both_directions() {
        let sent = Msg {
            send_id: "u1".into(),
            receiver_id: "u2".into(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        };
        let received = Msg {
            send_id: "u2".into(),
            receiver_id: "u1".into(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        };
        assert_eq!(conversation_id_of(&sent), "d:u1:u2");
        assert_eq!(conversation_id_of(&received), "d:u1:u2");
    }

    #[test]
    fn conversation_id_of_group_prefers_group_id() {
        let msg = Msg {
            send_id: "u1".into(),
            receiver_id: "g1".into(),
            group_id: "g1".into(),
            msg_type: MsgType::GroupMsg as i32,
            ..Default::default()
        };
        assert_eq!(conversation_id_of(&msg), "g:g1");
        assert_eq!(conv_type_of(&msg), 2);
    }

    #[test]
    fn conversation_id_of_group_falls_back_to_receiver() {
        let msg = Msg {
            send_id: "u1".into(),
            receiver_id: "g9".into(),
            group_id: String::new(),
            msg_type: MsgType::GroupMsg as i32,
            ..Default::default()
        };
        assert_eq!(conversation_id_of(&msg), "g:g9");
    }

    #[test]
    fn peer_resolution() {
        assert_eq!(peer_of_single("d:u1:u2", "u1"), Some("u2".into()));
        assert_eq!(peer_of_single("d:u1:u2", "u2"), Some("u1".into()));
        assert_eq!(peer_of_single("d:u1:u2", "u3"), None);
        assert_eq!(peer_of_single("g:g1", "u1"), None);
    }

    #[test]
    fn inline_decision_respects_threshold() {
        let mut msg = Msg::default();
        msg.content = vec![0u8; 100];
        let entry = MsgEntry::new("d:u1:u2", 1, msg);
        assert!(entry.should_inline(100));
        assert!(!entry.should_inline(99));
    }
}
