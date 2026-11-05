//! 消息存储层：统一读扩散
//!
//! 一条消息只写一次，写入成本与群规模无关 —— 单聊与十万人群同价。
//! 表结构见 `docs/MESSAGES_STORAGE_DDL.sql`，接口见 [`read_spread`]。
//!
//! 与已移除的写扩散方案的区别：不存在 user 维度的消息副本，
//! 拉取按 `(conversation_id, seq)` 主键扫描，已读是游标而非行级标记。

use std::sync::Arc;

use ::sqlx::PgPool;

use cache::SeqCache;
use contract::{error::Error, proto::message::Msg};
use runtime::config::AppConfig;

mod postgres;

pub mod read_spread;

/// 读扩散仓储集合
///
/// 对应 `conversation` / `msg_meta` / `msg_content` / `user_conversation` 四张表。
#[derive(Debug)]
pub struct ReadSpreadRepo {
    /// 会话序号与元信息
    pub conversation: Box<dyn read_spread::ConversationRepo>,
    /// 消息元数据与正文
    pub meta: Box<dyn read_spread::MsgMetaRepo>,
    /// 用户已读游标
    pub user_conversation: Box<dyn read_spread::UserConversationRepo>,
}

impl ReadSpreadRepo {
    pub fn new(pool: PgPool) -> Self {
        Self {
            conversation: Box::new(postgres::PgConversationRepo::new(pool.clone())),
            meta: Box::new(postgres::PgMsgMetaRepo::new(pool.clone())),
            user_conversation: Box::new(postgres::PgUserConversationRepo::new(pool)),
        }
    }

    /// 按配置建立独立连接池，并校验表结构
    ///
    /// 每个调用方应各自持有一份，不复用其他模块的池：
    /// 历史拉取的长查询不应挤占消息写入与序号分配。
    pub async fn from_config(config: &AppConfig) -> Result<Self, Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.database.postgres.effective_pool_size())
            .connect(&config.database.pg_url())
            .await?;
        Self::verify_schema(&pool).await?;
        Ok(Self::new(pool))
    }

    /// 启动时校验核心表是否存在
    ///
    /// 用 `to_regclass` 而不是 `pg_tables`：分区表的父表 relkind 是 `p`，
    /// 不会出现在 `pg_tables` 里，会被误判为缺表。
    ///
    /// 缺表时直接给出可执行的指引，而不是等到第一条消息处理时才抛 SQL 错误。
    async fn verify_schema(pool: &PgPool) -> Result<(), Error> {
        const REQUIRED: [&str; 4] = [
            "conversation",
            "msg_meta",
            "msg_content",
            "user_conversation",
        ];

        let names: Vec<String> = REQUIRED.iter().map(|s| s.to_string()).collect();

        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT name, to_regclass(name) IS NOT NULL AS present
             FROM unnest($1::text[]) AS name",
        )
        .bind(&names)
        .fetch_all(pool)
        .await?;

        let missing: Vec<&str> = rows
            .iter()
            .filter(|(_, present)| !present)
            .map(|(name, _)| name.as_str())
            .collect();

        if !missing.is_empty() {
            return Err(Error::Internal(format!(
                "消息存储表缺失: {}。请先执行 docs/MESSAGES_STORAGE_DDL.sql",
                missing.join(", ")
            )));
        }

        Ok(())
    }
}

/// 读扩散消息写入器
///
/// 把「幂等检查 → 分配序号 → 写元数据 → 登记成员 → 更新会话水位」
/// 收敛成一次调用。
#[derive(Debug)]
pub struct MessageWriter {
    repo: Arc<ReadSpreadRepo>,
    cache: Arc<dyn SeqCache>,
    inline_max_bytes: usize,
}

impl MessageWriter {
    pub fn new(
        repo: Arc<ReadSpreadRepo>,
        cache: Arc<dyn SeqCache>,
        inline_max_bytes: usize,
    ) -> Self {
        Self {
            repo,
            cache,
            inline_max_bytes,
        }
    }

    /// 领取会话序号
    ///
    /// Redis 号段优先，热群不再对 `conversation` 行加锁；
    /// 进入新号段时把「已预留上界」回写 PG，Redis 丢失后据此重建，
    /// 宁可跳号也不会重复分配。
    async fn next_seq(&self, conversation_id: &str, conv_type: i16) -> Result<i64, Error> {
        // 号段首次使用时，用 PG 中已持久化的水位做基线，避免从 1 重新开始
        if !self.cache.conversation_seq_exists(conversation_id).await? {
            let state = self
                .repo
                .conversation
                .ensure(conversation_id, conv_type)
                .await?;

            let reserved = self
                .cache
                .init_conversation_seq(conversation_id, state.last_seq)
                .await?;

            // 立即持久化本次预留的上界。
            // 否则 Redis 丢失后会从旧水位重新分配，已用过的序号会被再发一次。
            self.repo
                .conversation
                .flush_seq(conversation_id, reserved)
                .await?;
        }

        let (cur_seq, max_seq, need_flush) = self
            .cache
            .incr_conversation_seq(conversation_id)
            .await?;

        if need_flush {
            self.repo
                .conversation
                .flush_seq(conversation_id, max_seq)
                .await?;
        }

        Ok(cur_seq)
    }

    /// 保存一条消息
    ///
    /// 返回 `(conversation_id, seq, 是否首次写入)`。
    /// 重复消息（重放）不会分配新序号，直接返回已有 seq，
    /// 避免重放把会话水位推高、污染未读计算。
    ///
    /// `members` 为该会话的参与者：单聊传双方，群聊传全部成员，
    /// 用于登记 `user_conversation`（未读与已读游标的基础）。
    pub async fn save(
        &self,
        msg: Msg,
        members: &[String],
    ) -> Result<(String, i64, bool), Error> {
        let conversation_id = read_spread::conversation_id_of(&msg);
        let conv_type = read_spread::conv_type_of(&msg);

        if !msg.server_id.is_empty() {
            if let Some(existing) = self.repo.meta.get(&conversation_id, &msg.server_id).await? {
                return Ok((conversation_id, existing.seq, false));
            }
        }

        let seq = self.next_seq(&conversation_id, conv_type).await?;

        let entry = read_spread::MsgEntry::new(conversation_id.clone(), seq, msg.clone());
        let inserted = self.repo.meta.insert(&entry, self.inline_max_bytes).await?;

        if !members.is_empty() {
            self.repo
                .user_conversation
                .ensure_members(members, &conversation_id)
                .await?;
        }

        self.repo
            .conversation
            .touch(&conversation_id, seq, msg.send_time)
            .await?;

        Ok((conversation_id, seq, inserted))
    }
}
