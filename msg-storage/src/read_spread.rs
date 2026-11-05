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
use contract::proto::message::Msg;

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

    /// 批量更新会话消息水位
    ///
    /// 参数为 `(conversation_id, last_msg_seq, last_msg_time)`，
    /// 同一会话出现多次时由调用方汇总成最大值。
    ///
    /// 每条消息都更新一次 `conversation` 行会让同一批热点行被高频改写：
    /// 3000 msg/s、64 个会话时就是 3000 次/秒的行更新，WAL 与死元组
    /// 都按消息数线性增长，而这条链路只需要「最终前进到最大值」。
    /// 按批汇总后一次提交，语义由 `GREATEST` 保证只增不减。
    async fn touch_batch(&self, watermarks: &[(String, i64, i64)]) -> Result<(), Error>;

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

    /// 用一条数据修改 CTE 完成消息写入与参与者登记
    ///
    /// 返回 `(是否首次写入, 新写入的 seq)`。原先这几步是多个独立 PG 调用，
    /// 每条消息要多花数次网络往返。
    ///
    /// 会话水位不在这里更新：那是同一行被反复改写的热点写入，
    /// 改由 [`ConversationRepo::touch_batch`] 按批汇总一次提交。
    async fn insert_with_participants(
        &self,
        entry: &MsgEntry,
        members: &[String],
        inline_max_bytes: usize,
    ) -> Result<(bool, Option<i64>), Error>;

    /// 批量写入（同一事务），返回实际写入条数
    async fn insert_batch(&self, entries: &[MsgEntry], inline_max_bytes: usize)
        -> Result<u64, Error>;

    /// 批量判重：返回该会话内已存在的 `(server_id, seq)`
    ///
    /// 替代「每条消息一次 SELECT」。语义不变——重复投递仍然不消耗号段，
    /// 只是把 N 次往返压成一次。调用方据返回值过滤掉重放消息后再领号。
    async fn find_existing(
        &self,
        conversation_id: &str,
        server_ids: &[String],
    ) -> Result<Vec<(String, i64)>, Error>;

    /// 同一会话的多条消息用一条语句写入
    ///
    /// 返回实际插入的 `(server_id, seq)`。并发下被别的消费者抢先写过的行
    /// 由唯一约束挡掉，不会出现在返回值里——调用方据此跳过重复推送。
    ///
    /// 与 [`Self::insert_with_participants`] 的差别只在语句条数：
    /// 那是「一条消息一条语句」，这里是「一批消息一条语句」。
    /// 每条语句在本机要花约 0.25 核毫秒，语句数就是满载吞吐的上限。
    async fn insert_batch_with_participants(
        &self,
        conversation_id: &str,
        entries: &[MsgEntry],
        members: &[String],
        inline_max_bytes: usize,
    ) -> Result<Vec<(String, i64)>, Error>;

    /// 跨会话批量判重：返回已存在的 `(conversation_id, server_id, seq)`
    ///
    /// 一条语句覆盖整批，代替「每个会话一次 SELECT」。
    /// 分区键随参数给出，PG 按值裁剪到相关分区。
    async fn find_existing_multi(
        &self,
        pairs: &[(String, String)],
    ) -> Result<Vec<(String, String, i64)>, Error>;

    /// 跨会话批量写入：整批一条语句
    ///
    /// `entries` 里每条自带会话 id，`members` 是 `(user_id, conversation_id)` 对。
    /// 返回实际插入的 `(conversation_id, server_id, seq)`。
    ///
    /// 这是「按会话分批」的升级：会话分批时每批仍有「会话数 × 2」条语句，
    /// 只有跨会话合并才能把语句数从「每消息」压到「每批」。
    async fn insert_batch_multi(
        &self,
        entries: &[MsgEntry],
        members: &[(String, String)],
        inline_max_bytes: usize,
    ) -> Result<Vec<(String, String, i64)>, Error>;

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
// 会话键同时决定 Kafka 分区、落库序号与网关上行保序，推导只保留一份实现：
// 放在 contract（所有 crate 共享），此处 re-export 维持既有调用路径。
pub use contract::types::conversation::{
    conv_type_of, conversation_id_of, group_conversation_id, peer_of_single,
    single_conversation_id,
};

#[cfg(test)]
mod tests {
    use super::*;
    use contract::proto::message::MsgType;

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
