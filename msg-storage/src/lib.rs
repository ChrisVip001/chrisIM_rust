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
use runtime::stage_profile::{Lap, Stage};

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
        // 热路径只有一次 Redis 往返；号段未初始化时返回 0，走冷启动。
        let (cur_seq, max_seq, need_flush) = self
            .cache
            .incr_conversation_seq(conversation_id)
            .await?;

        let (cur_seq, max_seq, need_flush) = if cur_seq == 0 {
            // 号段首次使用：用 PG 中已持久化的水位做基线，避免 Redis 丢失后从 1 重新分配
            let state = self
                .repo
                .conversation
                .ensure(conversation_id, conv_type)
                .await?;
            let reserved = self
                .cache
                .init_conversation_seq(conversation_id, state.last_seq)
                .await?;
            // 立即持久化本次预留的上界，否则 Redis 丢失后会重复分配已用过的序号
            self.repo
                .conversation
                .flush_seq(conversation_id, reserved)
                .await?;
            self.cache.incr_conversation_seq(conversation_id).await?
        } else {
            (cur_seq, max_seq, need_flush)
        };

        if cur_seq == 0 {
            return Err(Error::Internal(format!(
                "会话号段冷启动后仍不可用: {}",
                conversation_id
            )));
        }

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
    /// 写入由单条 CTE 完成（元数据/正文 + 参与者 + 水位），并发重复时
    /// 读回数据库里的权威序号，且 CTE 不会更新会话水位。
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

        // 分段计时：幂等查询 / 号段 / 写入三段，用于定位落库延迟来源
        let mut lap = Lap::start();

        // 幂等预查询必须保留：重复投递不消耗号段、不推高水位。
        if !msg.server_id.is_empty() {
            let existing = self.repo.meta.get(&conversation_id, &msg.server_id).await?;
            lap.lap(Stage::Idem);
            if let Some(existing) = existing {
                return Ok((conversation_id, existing.seq, false));
            }
        }

        let seq = self.next_seq(&conversation_id, conv_type).await?;
        // 号段耗时只上报给采样器，单条路径不需要它参与慢日志
        lap.lap(Stage::Seq);

        let entry = read_spread::MsgEntry::new(conversation_id.clone(), seq, msg.clone());
        let (inserted, written_seq) = self
            .repo
            .meta
            .insert_with_participants(&entry, members, self.inline_max_bytes)
            .await?;
        lap.lap(Stage::Insert);

        if inserted {
            return Ok((conversation_id, written_seq.unwrap_or(seq), true));
        }

        let existing = self.repo.meta.get(&conversation_id, &msg.server_id).await?;
        Ok((
            conversation_id,
            existing.map(|m| m.seq).unwrap_or(seq),
            false,
        ))
    }

    /// 批次级会话水位回写
    ///
    /// 参数为 `(conversation_id, seq, send_time)`，同一会话可以出现多次，
    /// 库侧用 `GREATEST` 只前进。调用方在整批消息落库成功后调用一次，
    /// 把「每条消息一次热点行更新」降为「每个会话一批一次」。
    ///
    /// 代价是水位最多滞后一个批次（几十毫秒）：会话列表的最后一条消息
    /// 可能晚一个批次才刷新，消息本身已经落库并由推送即时送达。
    pub async fn touch_watermarks(&self, watermarks: &[(String, i64, i64)]) -> Result<(), Error> {
        self.repo.conversation.touch_batch(watermarks).await
    }

    /// 批量保存同一会话的多条消息
    ///
    /// 调用方保证 `msgs` 属于同一会话且按到达顺序排列——顺序即序号顺序。
    /// 返回与 `msgs` 等长的 `(server_id, seq, 是否首次写入)`。
    ///
    /// 相对逐条 [`Self::save`]，数据库语句从「每消息 2 条」降到「每批 2 条」：
    /// 一次批量判重 + 一次多行写入。本机每条语句约 0.25 核毫秒，
    /// 语句数直接决定满载吞吐上限，所以这是整条链路最有效的一刀。
    ///
    /// 语义与逐条一致：重复投递不消耗号段（判重先行），并发抢先由唯一约束兜底。
    pub async fn save_batch(
        &self,
        msgs: Vec<Msg>,
        members: &[String],
    ) -> Result<Vec<(String, i64, bool)>, Error> {
        if msgs.is_empty() {
            return Ok(Vec::new());
        }

        // 同一组即同一会话，会话键取第一条即可
        let conversation_id = read_spread::conversation_id_of(&msgs[0]);
        let conv_type = read_spread::conv_type_of(&msgs[0]);

        let mut lap = Lap::start();

        // 1. 一次判重查询代替每条一次 SELECT
        let lookup: Vec<String> = msgs
            .iter()
            .filter(|m| !m.server_id.is_empty())
            .map(|m| m.server_id.clone())
            .collect();
        let existing: std::collections::HashMap<String, i64> = self
            .repo
            .meta
            .find_existing(&conversation_id, &lookup)
            .await?
            .into_iter()
            .collect();
        lap.lap(Stage::Idem);

        // 2. 未出现过的消息按顺序领号：顺序即序号顺序，保序靠调用方
        let mut result: Vec<(String, i64, bool)> = Vec::with_capacity(msgs.len());
        let mut pending: Vec<read_spread::MsgEntry> = Vec::with_capacity(msgs.len());
        let mut pending_slots: Vec<usize> = Vec::with_capacity(msgs.len());

        for msg in msgs {
            if let Some(&seq) = existing.get(&msg.server_id) {
                result.push((msg.server_id, seq, false));
                continue;
            }

            let seq = self.next_seq(&conversation_id, conv_type).await?;
            pending_slots.push(result.len());
            result.push((msg.server_id.clone(), seq, false));
            pending.push(read_spread::MsgEntry::new(
                conversation_id.clone(),
                seq,
                msg,
            ));
        }
        // 号段耗时只上报给采样器，这里的批量慢日志只看写入段
        lap.lap(Stage::Seq);

        // 3. 一条多行语句写入整批
        if !pending.is_empty() {
            let inserted = self
                .repo
                .meta
                .insert_batch_with_participants(
                    &conversation_id,
                    &pending,
                    members,
                    self.inline_max_bytes,
                )
                .await?;
            let inserted: std::collections::HashMap<String, i64> =
                inserted.into_iter().collect();

            for slot in pending_slots {
                let server_id = result[slot].0.clone();
                // 不在返回值里 = 被并发消费者抢先写入了，视为重复
                if let Some(&seq) = inserted.get(&server_id) {
                    result[slot].1 = seq;
                    result[slot].2 = true;
                }
            }
        }
        lap.lap(Stage::Insert);

        Ok(result)
    }

    /// 跨会话批量保存
    ///
    /// `items` 为 `(消息, 该会话的参与者)`。返回与入参等长的
    /// `(conversation_id, seq, 是否首次写入)`。
    ///
    /// 与 [`Self::save_batch`] 的差别是批量边界：那个按会话分批，
    /// 每批仍有「会话数 × 2」条语句；这里整批一条判重 + 一条写入，
    /// 语句数与消息数无关。满载时这是决定 PG 负载的唯一变量。
    pub async fn save_batch_multi(
        &self,
        items: Vec<(Msg, Vec<String>)>,
    ) -> Result<Vec<(String, i64, bool)>, Error> {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        let started = std::time::Instant::now();
        let mut lap = Lap::start();

        // 1. 一次跨会话判重
        let pairs: Vec<(String, String)> = items
            .iter()
            .filter(|(m, _)| !m.server_id.is_empty())
            .map(|(m, _)| (read_spread::conversation_id_of(m), m.server_id.clone()))
            .collect();
        let existing: std::collections::HashMap<(String, String), i64> = self
            .repo
            .meta
            .find_existing_multi(&pairs)
            .await?
            .into_iter()
            .map(|(c, s, seq)| ((c, s), seq))
            .collect();
        let idem = lap.lap(Stage::Idem);

        // 2. 未出现过的按会话分组领号：组内串行保序，组间并发。
        //    整批串行领号时号段是最大的单点（每批几十次往返串成一串）。
        let mut result: Vec<(String, i64, bool)> = Vec::with_capacity(items.len());
        let mut pending: Vec<Option<read_spread::MsgEntry>> =
            (0..items.len()).map(|_| None).collect();
        let mut members: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        // 待领号的消息按会话归并，保留其在 items 中的下标
        let mut groups: Vec<(String, i16, Vec<(usize, Msg)>)> = Vec::new();
        let mut group_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for (idx, (msg, participants)) in items.into_iter().enumerate() {
            let conversation_id = read_spread::conversation_id_of(&msg);

            if let Some(&seq) = existing.get(&(conversation_id.clone(), msg.server_id.clone())) {
                result.push((conversation_id, seq, false));
                continue;
            }

            for user in &participants {
                members.insert((user.clone(), conversation_id.clone()));
            }

            // 先占位，领号成功后回填
            result.push((conversation_id.clone(), 0, false));

            let conv_type = read_spread::conv_type_of(&msg);
            match group_index.get(&conversation_id) {
                Some(&i) => groups[i].2.push((idx, msg)),
                None => {
                    group_index.insert(conversation_id.clone(), groups.len());
                    groups.push((conversation_id, conv_type, vec![(idx, msg)]));
                }
            }
        }

        let allocated = futures::future::join_all(groups.iter().map(|(conv, conv_type, msgs)| {
            let conv = conv.clone();
            let conv_type = *conv_type;
            async move {
                let mut out = Vec::with_capacity(msgs.len());
                for (idx, _) in msgs {
                    let seq = self.next_seq(&conv, conv_type).await?;
                    out.push((*idx, seq));
                }
                Ok::<_, Error>(out)
            }
        }))
        .await;

        let mut assigned: Vec<(usize, i64)> = Vec::with_capacity(result.len());
        for allocated in allocated {
            assigned.extend(allocated?);
        }
        let assigned: std::collections::HashMap<usize, i64> = assigned.into_iter().collect();

        for (conv, _, msgs) in &groups {
            for (idx, msg) in msgs {
                if let Some(&seq) = assigned.get(idx) {
                    result[*idx].1 = seq;
                    pending[*idx] = Some(read_spread::MsgEntry::new(
                        conv.clone(),
                        seq,
                        msg.clone(),
                    ));
                }
            }
        }
        let seq_time = lap.lap(Stage::Seq);

        // 3. 一条多行语句写入整批；下标与 result 一一对应
        if pending.iter().any(|p| p.is_some()) {
            let members: Vec<(String, String)> = members.into_iter().collect();
            let entries: Vec<read_spread::MsgEntry> = pending.iter().flatten().cloned().collect();
            let inserted = self
                .repo
                .meta
                .insert_batch_multi(&entries, &members, self.inline_max_bytes)
                .await?;
            let inserted: std::collections::HashMap<(String, String), i64> = inserted
                .into_iter()
                .map(|(c, s, seq)| ((c, s), seq))
                .collect();

            for (idx, entry) in pending.iter().enumerate() {
                let Some(entry) = entry else { continue };
                let key = (
                    entry.conversation_id.clone(),
                    entry.msg.server_id.clone(),
                );
                if let Some(&seq) = inserted.get(&key) {
                    result[idx].1 = seq;
                    result[idx].2 = true;
                }
            }
        }
        let insert = lap.lap(Stage::Insert);

        let total = started.elapsed();
        if total > std::time::Duration::from_millis(100) {
            ::tracing::warn!(
                "批量落库耗时异常: 总={:?} 判重={:?} 领号={:?} 写入={:?} 条数={}",
                total,
                idem,
                seq_time,
                insert,
                result.len()
            );
        }

        Ok(result)
    }
}
