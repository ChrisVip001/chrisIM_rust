use async_trait::async_trait;
use sqlx::{PgPool, Row};

use contract::error::Error;
use contract::proto::message::Msg;

use crate::read_spread::{
    ConversationRepo, ConversationState, MsgEntry, MsgMetaRepo, UserConversationRepo,
    UserConversationState,
};

/// 会话仓库（`conversation` 表）
#[derive(Debug)]
pub struct PgConversationRepo {
    pool: PgPool,
}

impl PgConversationRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ConversationRepo for PgConversationRepo {
    async fn next_seq(&self, conversation_id: &str, conv_type: i16) -> Result<i64, Error> {
        // 单条 SQL 完成「不存在则建、存在则自增」，避免先查后改的竞态。
        // 热群会在此处产生行锁争用，后续由 Redis 号段承担分配、此处只做水位回写。
        let row = sqlx::query(
            "INSERT INTO conversation (conversation_id, conv_type, last_seq)
             VALUES ($1, $2, 1)
             ON CONFLICT (conversation_id)
             DO UPDATE SET last_seq = conversation.last_seq + 1, updated_at = NOW()
             RETURNING last_seq",
        )
        .bind(conversation_id)
        .bind(conv_type)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.try_get::<i64, _>("last_seq")?)
    }

    async fn ensure(
        &self,
        conversation_id: &str,
        conv_type: i16,
    ) -> Result<ConversationState, Error> {
        let row = sqlx::query(
            "INSERT INTO conversation (conversation_id, conv_type)
             VALUES ($1, $2)
             ON CONFLICT (conversation_id)
             DO UPDATE SET conv_type = conversation.conv_type
             RETURNING conversation_id, conv_type, last_seq, last_msg_seq, last_msg_time",
        )
        .bind(conversation_id)
        .bind(conv_type)
        .fetch_one(&self.pool)
        .await?;

        Ok(row_to_conversation(&row)?)
    }

    async fn get(&self, conversation_id: &str) -> Result<Option<ConversationState>, Error> {
        let row = sqlx::query(
            "SELECT conversation_id, conv_type, last_seq, last_msg_seq, last_msg_time
             FROM conversation WHERE conversation_id = $1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(row_to_conversation(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_many(&self, conversation_ids: &[String]) -> Result<Vec<ConversationState>, Error> {
        if conversation_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            "SELECT conversation_id, conv_type, last_seq, last_msg_seq, last_msg_time
             FROM conversation WHERE conversation_id = ANY($1)",
        )
        .bind(conversation_ids)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_conversation).collect()
    }

    async fn flush_seq(&self, conversation_id: &str, last_seq: i64) -> Result<(), Error> {
        sqlx::query(
            "UPDATE conversation SET last_seq = GREATEST(last_seq, $2), updated_at = NOW()
             WHERE conversation_id = $1",
        )
        .bind(conversation_id)
        .bind(last_seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn touch(
        &self,
        conversation_id: &str,
        last_msg_seq: i64,
        last_msg_time: i64,
    ) -> Result<(), Error> {
        sqlx::query(
            "UPDATE conversation
             SET last_msg_seq = GREATEST(last_msg_seq, $2),
                 last_msg_time = GREATEST(last_msg_time, $3),
                 updated_at = NOW()
             WHERE conversation_id = $1",
        )
        .bind(conversation_id)
        .bind(last_msg_seq)
        .bind(last_msg_time)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn touch_batch(&self, watermarks: &[(String, i64, i64)]) -> Result<(), Error> {
        if watermarks.is_empty() {
            return Ok(());
        }

        // 用 unnest 三列数组做一次表连接更新：一次往返、一条语句，
        // 会话数就是语句里的行数，与消息条数无关。
        let (ids, seqs, times): (Vec<String>, Vec<i64>, Vec<i64>) = watermarks
            .iter()
            .fold(
                (
                    Vec::with_capacity(watermarks.len()),
                    Vec::with_capacity(watermarks.len()),
                    Vec::with_capacity(watermarks.len()),
                ),
                |(mut ids, mut seqs, mut times), (id, seq, time)| {
                    ids.push(id.clone());
                    seqs.push(*seq);
                    times.push(*time);
                    (ids, seqs, times)
                },
            );

        sqlx::query(
            "UPDATE conversation c
             SET last_msg_seq = GREATEST(c.last_msg_seq, v.seq),
                 last_msg_time = GREATEST(c.last_msg_time, v.time),
                 updated_at = NOW()
             FROM unnest($1::varchar[], $2::bigint[], $3::bigint[]) AS v(cid, seq, time)
             WHERE c.conversation_id = v.cid",
        )
        .bind(&ids)
        .bind(&seqs)
        .bind(&times)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete(&self, conversation_id: &str) -> Result<(), Error> {
        sqlx::query("DELETE FROM conversation WHERE conversation_id = $1")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn row_to_conversation(row: &sqlx::postgres::PgRow) -> Result<ConversationState, Error> {
    Ok(ConversationState {
        conversation_id: row.try_get("conversation_id")?,
        conv_type: row.try_get("conv_type")?,
        last_seq: row.try_get("last_seq")?,
        last_msg_seq: row.try_get("last_msg_seq")?,
        last_msg_time: row.try_get("last_msg_time")?,
    })
}

// ---------------------------------------------------------------------------
// 消息元数据 + 正文
// ---------------------------------------------------------------------------

/// 消息仓库（`msg_meta` + `msg_content`）
#[derive(Debug)]
pub struct PgMsgMetaRepo {
    pool: PgPool,
}

impl PgMsgMetaRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MsgMetaRepo for PgMsgMetaRepo {
    async fn insert(&self, entry: &MsgEntry, inline_max_bytes: usize) -> Result<bool, Error> {
        let inline = entry.should_inline(inline_max_bytes);
        let mut tx = self.pool.begin().await?;

        let result = sqlx::query(
            "INSERT INTO msg_meta
                (conversation_id, seq, server_id, send_id, msg_type, content_type,
                 send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11, $12, $13)
             ON CONFLICT (conversation_id, server_id) DO NOTHING",
        )
        .bind(&entry.conversation_id)
        .bind(entry.seq)
        .bind(&entry.msg.server_id)
        .bind(&entry.msg.send_id)
        .bind(entry.msg.msg_type)
        .bind(entry.msg.content_type)
        .bind(entry.msg.send_time)
        .bind(entry.msg.content.len() as i32)
        .bind(if inline { Some(&entry.msg.content) } else { None })
        .bind(detail_json(&entry.msg))
        .bind(entry.msg.is_revoked)
        .bind(entry.msg.revoke_time)
        .bind(&entry.msg.revoked_by)
        .execute(&mut *tx)
        .await?;

        // 重复消息（重放）不写正文，保持幂等
        if result.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        if !inline {
            sqlx::query(
                "INSERT INTO msg_content (server_id, content) VALUES ($1, $2)
                 ON CONFLICT (server_id) DO NOTHING",
            )
            .bind(&entry.msg.server_id)
            .bind(&entry.msg.content)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(true)
    }

    async fn insert_with_participants(
        &self,
        entry: &MsgEntry,
        members: &[String],
        inline_max_bytes: usize,
    ) -> Result<(bool, Option<i64>), Error> {
        let inline = entry.should_inline(inline_max_bytes);

        // 外置正文只在元数据真正插入时写入，重复投递保持幂等
        let body_cte = if inline {
            ""
        } else {
            ", body AS (
                 INSERT INTO msg_content (server_id, content)
                 SELECT $3, $15 WHERE EXISTS (SELECT 1 FROM ins)
                 ON CONFLICT (server_id) DO NOTHING
             )"
        };
        let sql = format!(
            "WITH ins AS (
                 INSERT INTO msg_meta
                     (conversation_id, seq, server_id, send_id, msg_type, content_type,
                      send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::jsonb,$11,$12,$13)
                 ON CONFLICT (conversation_id, server_id) DO NOTHING
                 RETURNING seq
             ){body}
             , mem AS (
                 INSERT INTO user_conversation (user_id, conversation_id)
                 SELECT u, $1 FROM unnest($14::varchar[]) AS u
                 ON CONFLICT (user_id, conversation_id) DO NOTHING
             )
             SELECT seq FROM ins",
            body = body_cte,
        );

        let query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(&entry.conversation_id)
            .bind(entry.seq)
            .bind(&entry.msg.server_id)
            .bind(&entry.msg.send_id)
            .bind(entry.msg.msg_type)
            .bind(entry.msg.content_type)
            .bind(entry.msg.send_time)
            .bind(entry.msg.content.len() as i32)
            .bind(if inline { Some(&entry.msg.content) } else { None })
            .bind(detail_json(&entry.msg))
            .bind(entry.msg.is_revoked)
            .bind(entry.msg.revoke_time)
            .bind(&entry.msg.revoked_by)
            .bind(members.to_vec());
        let query = if inline {
            query
        } else {
            query.bind(&entry.msg.content)
        };

        let row = query.fetch_optional(&self.pool).await?;
        Ok((row.is_some(), row.map(|(seq,)| seq)))
    }

    async fn insert_batch(
        &self,
        entries: &[MsgEntry],
        inline_max_bytes: usize,
    ) -> Result<u64, Error> {
        if entries.is_empty() {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await?;
        let mut written = 0u64;

        // 逐条仍在同一事务内，网络往返一次；
        // 真正的多行单语句批量写需要 unnest 数组参数，留待批处理通道接入。
        for entry in entries {
            let inline = entry.should_inline(inline_max_bytes);
            let result = sqlx::query(
                "INSERT INTO msg_meta
                    (conversation_id, seq, server_id, send_id, msg_type, content_type,
                     send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11, $12, $13)
                 ON CONFLICT (conversation_id, server_id) DO NOTHING",
            )
            .bind(&entry.conversation_id)
            .bind(entry.seq)
            .bind(&entry.msg.server_id)
            .bind(&entry.msg.send_id)
            .bind(entry.msg.msg_type)
            .bind(entry.msg.content_type)
            .bind(entry.msg.send_time)
            .bind(entry.msg.content.len() as i32)
            .bind(if inline { Some(&entry.msg.content) } else { None })
            .bind(detail_json(&entry.msg))
            .bind(entry.msg.is_revoked)
            .bind(entry.msg.revoke_time)
            .bind(&entry.msg.revoked_by)
            .execute(&mut *tx)
            .await?;

            if result.rows_affected() == 0 {
                continue;
            }
            written += 1;

            if !inline {
                sqlx::query(
                    "INSERT INTO msg_content (server_id, content) VALUES ($1, $2)
                     ON CONFLICT (server_id) DO NOTHING",
                )
                .bind(&entry.msg.server_id)
                .bind(&entry.msg.content)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(written)
    }

    async fn find_existing(
        &self,
        conversation_id: &str,
        server_ids: &[String],
    ) -> Result<Vec<(String, i64)>, Error> {
        if server_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as::<_, (String, i64)>(
            "SELECT server_id, seq FROM msg_meta
             WHERE conversation_id = $1 AND server_id = ANY($2::varchar[])",
        )
        .bind(conversation_id)
        .bind(server_ids.to_vec())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn insert_batch_with_participants(
        &self,
        conversation_id: &str,
        entries: &[MsgEntry],
        members: &[String],
        inline_max_bytes: usize,
    ) -> Result<Vec<(String, i64)>, Error> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // 列式参数：每列一个数组，由 unnest 按位置展开成多行。
        // 这样整批只是一条语句、一次往返；参数个数固定，与批量大小无关。
        let n = entries.len();
        let mut seqs: Vec<i64> = Vec::with_capacity(n);
        let mut server_ids: Vec<String> = Vec::with_capacity(n);
        let mut send_ids: Vec<String> = Vec::with_capacity(n);
        let mut msg_types: Vec<i32> = Vec::with_capacity(n);
        let mut content_types: Vec<i32> = Vec::with_capacity(n);
        let mut send_times: Vec<i64> = Vec::with_capacity(n);
        let mut content_sizes: Vec<i32> = Vec::with_capacity(n);
        let mut inline_contents: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
        let mut details: Vec<String> = Vec::with_capacity(n);
        let mut is_revokeds: Vec<bool> = Vec::with_capacity(n);
        let mut revoke_times: Vec<i64> = Vec::with_capacity(n);
        let mut revoked_bys: Vec<String> = Vec::with_capacity(n);

        // 只有超大正文需要外置到 msg_content，单独两个数组
        let mut body_ids: Vec<String> = Vec::new();
        let mut body_contents: Vec<Vec<u8>> = Vec::new();

        for entry in entries {
            let inline = entry.should_inline(inline_max_bytes);
            seqs.push(entry.seq);
            server_ids.push(entry.msg.server_id.clone());
            send_ids.push(entry.msg.send_id.clone());
            msg_types.push(entry.msg.msg_type);
            content_types.push(entry.msg.content_type);
            send_times.push(entry.msg.send_time);
            content_sizes.push(entry.msg.content.len() as i32);
            inline_contents.push(if inline {
                Some(entry.msg.content.clone())
            } else {
                None
            });
            details.push(detail_json(&entry.msg));
            is_revokeds.push(entry.msg.is_revoked);
            revoke_times.push(entry.msg.revoke_time);
            revoked_bys.push(entry.msg.revoked_by.clone());

            if !inline {
                body_ids.push(entry.msg.server_id.clone());
                body_contents.push(entry.msg.content.clone());
            }
        }

        // body 用 JOIN ins 限定：并发下没插进去的行不写孤儿正文
        let rows = sqlx::query_as::<_, (String, i64)>(
            "WITH ins AS (
                 INSERT INTO msg_meta
                     (conversation_id, seq, server_id, send_id, msg_type, content_type,
                      send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
                 SELECT $1, t.seq, t.server_id, t.send_id, t.msg_type, t.content_type,
                        t.send_time, t.content_size, t.inline_content, t.detail::jsonb,
                        t.is_revoked, t.revoke_time, t.revoked_by
                 FROM unnest(
                     $2::bigint[], $3::varchar[], $4::varchar[], $5::int[], $6::int[],
                     $7::bigint[], $8::int[], $9::bytea[], $10::text[], $11::bool[],
                     $12::bigint[], $13::varchar[]
                 ) AS t(seq, server_id, send_id, msg_type, content_type,
                        send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
                 ON CONFLICT (conversation_id, server_id) DO NOTHING
                 RETURNING server_id, seq
             )
             , body AS (
                 INSERT INTO msg_content (server_id, content)
                 SELECT b.sid, b.content
                 FROM unnest($14::varchar[], $15::bytea[]) AS b(sid, content)
                 WHERE b.sid IN (SELECT server_id FROM ins)
                 ON CONFLICT (server_id) DO NOTHING
             )
             , mem AS (
                 INSERT INTO user_conversation (user_id, conversation_id)
                 SELECT u, $1 FROM unnest($16::varchar[]) AS u
                 ON CONFLICT (user_id, conversation_id) DO NOTHING
             )
             SELECT server_id, seq FROM ins",
        )
        .bind(conversation_id)
        .bind(seqs)
        .bind(server_ids)
        .bind(send_ids)
        .bind(msg_types)
        .bind(content_types)
        .bind(send_times)
        .bind(content_sizes)
        .bind(inline_contents)
        .bind(details)
        .bind(is_revokeds)
        .bind(revoke_times)
        .bind(revoked_bys)
        .bind(body_ids)
        .bind(body_contents)
        .bind(members.to_vec())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn find_existing_multi(
        &self,
        pairs: &[(String, String)],
    ) -> Result<Vec<(String, String, i64)>, Error> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        let (conversations, server_ids): (Vec<String>, Vec<String>) = pairs
            .iter()
            .fold(
                (
                    Vec::with_capacity(pairs.len()),
                    Vec::with_capacity(pairs.len()),
                ),
                |(mut cs, mut ss), (c, s)| {
                    cs.push(c.clone());
                    ss.push(s.clone());
                    (cs, ss)
                },
            );

        // 用 unnest 驱动而不是 IN 列表：参数固定两个数组，
        // 分区键按值裁剪，语句数与批量大小无关。
        let rows = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT m.conversation_id, m.server_id, m.seq
             FROM unnest($1::varchar[], $2::varchar[]) AS p(cid, sid)
             JOIN msg_meta m ON m.conversation_id = p.cid AND m.server_id = p.sid",
        )
        .bind(conversations)
        .bind(server_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn insert_batch_multi(
        &self,
        entries: &[MsgEntry],
        members: &[(String, String)],
        inline_max_bytes: usize,
    ) -> Result<Vec<(String, String, i64)>, Error> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let n = entries.len();
        let mut conversations: Vec<String> = Vec::with_capacity(n);
        let mut seqs: Vec<i64> = Vec::with_capacity(n);
        let mut server_ids: Vec<String> = Vec::with_capacity(n);
        let mut send_ids: Vec<String> = Vec::with_capacity(n);
        let mut msg_types: Vec<i32> = Vec::with_capacity(n);
        let mut content_types: Vec<i32> = Vec::with_capacity(n);
        let mut send_times: Vec<i64> = Vec::with_capacity(n);
        let mut content_sizes: Vec<i32> = Vec::with_capacity(n);
        let mut inline_contents: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
        let mut details: Vec<String> = Vec::with_capacity(n);
        let mut is_revokeds: Vec<bool> = Vec::with_capacity(n);
        let mut revoke_times: Vec<i64> = Vec::with_capacity(n);
        let mut revoked_bys: Vec<String> = Vec::with_capacity(n);

        let mut body_ids: Vec<String> = Vec::new();
        let mut body_contents: Vec<Vec<u8>> = Vec::new();

        for entry in entries {
            let inline = entry.should_inline(inline_max_bytes);
            conversations.push(entry.conversation_id.clone());
            seqs.push(entry.seq);
            server_ids.push(entry.msg.server_id.clone());
            send_ids.push(entry.msg.send_id.clone());
            msg_types.push(entry.msg.msg_type);
            content_types.push(entry.msg.content_type);
            send_times.push(entry.msg.send_time);
            content_sizes.push(entry.msg.content.len() as i32);
            inline_contents.push(if inline {
                Some(entry.msg.content.clone())
            } else {
                None
            });
            details.push(detail_json(&entry.msg));
            is_revokeds.push(entry.msg.is_revoked);
            revoke_times.push(entry.msg.revoke_time);
            revoked_bys.push(entry.msg.revoked_by.clone());

            if !inline {
                body_ids.push(entry.msg.server_id.clone());
                body_contents.push(entry.msg.content.clone());
            }
        }

        let (member_users, member_convs): (Vec<String>, Vec<String>) = members.iter().fold(
            (
                Vec::with_capacity(members.len()),
                Vec::with_capacity(members.len()),
            ),
            |(mut us, mut cs), (u, c)| {
                us.push(u.clone());
                cs.push(c.clone());
                (us, cs)
            },
        );

        let rows = sqlx::query_as::<_, (String, String, i64)>(
            "WITH ins AS (
                 INSERT INTO msg_meta
                     (conversation_id, seq, server_id, send_id, msg_type, content_type,
                      send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
                 SELECT t.cid, t.seq, t.server_id, t.send_id, t.msg_type, t.content_type,
                        t.send_time, t.content_size, t.inline_content, t.detail::jsonb,
                        t.is_revoked, t.revoke_time, t.revoked_by
                 FROM unnest(
                     $1::varchar[], $2::bigint[], $3::varchar[], $4::varchar[], $5::int[], $6::int[],
                     $7::bigint[], $8::int[], $9::bytea[], $10::text[], $11::bool[],
                     $12::bigint[], $13::varchar[]
                 ) AS t(cid, seq, server_id, send_id, msg_type, content_type,
                        send_time, content_size, inline_content, detail, is_revoked, revoke_time, revoked_by)
                 -- 不指定冲突目标：同时兜住 (conversation_id, server_id) 唯一索引
                 -- 与主键 (conversation_id, seq)。只写一个目标时，撞主键会抛
                 -- unique_violation 让整批回滚，一条脏数据连坐同批所有会话。
                 ON CONFLICT DO NOTHING
                 RETURNING conversation_id, server_id, seq
             )
             , body AS (
                 INSERT INTO msg_content (server_id, content)
                 SELECT b.sid, b.content
                 FROM unnest($14::varchar[], $15::bytea[]) AS b(sid, content)
                 WHERE b.sid IN (SELECT server_id FROM ins)
                 ON CONFLICT (server_id) DO NOTHING
             )
             , mem AS (
                 INSERT INTO user_conversation (user_id, conversation_id)
                 SELECT m.uid, m.cid FROM unnest($16::varchar[], $17::varchar[]) AS m(uid, cid)
                 ON CONFLICT (user_id, conversation_id) DO NOTHING
             )
             SELECT conversation_id, server_id, seq FROM ins",
        )
        .bind(conversations)
        .bind(seqs)
        .bind(server_ids)
        .bind(send_ids)
        .bind(msg_types)
        .bind(content_types)
        .bind(send_times)
        .bind(content_sizes)
        .bind(inline_contents)
        .bind(details)
        .bind(is_revokeds)
        .bind(revoke_times)
        .bind(revoked_bys)
        .bind(body_ids)
        .bind(body_contents)
        .bind(member_users)
        .bind(member_convs)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn fetch_after(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<Msg>, Error> {
        let rows = sqlx::query(
            "SELECT server_id, seq, send_id, msg_type, content_type, send_time,
                    content_size, inline_content, detail::text AS detail,
                    is_revoked, revoke_time, revoked_by
             FROM msg_meta
             WHERE conversation_id = $1 AND seq > $2
             ORDER BY seq
             LIMIT $3",
        )
        .bind(conversation_id)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        self.assemble(conversation_id, rows).await
    }

    async fn fetch_range(
        &self,
        conversation_id: &str,
        start_seq: i64,
        end_seq: i64,
    ) -> Result<Vec<Msg>, Error> {
        let rows = sqlx::query(
            "SELECT server_id, seq, send_id, msg_type, content_type, send_time,
                    content_size, inline_content, detail::text AS detail,
                    is_revoked, revoke_time, revoked_by
             FROM msg_meta
             WHERE conversation_id = $1 AND seq BETWEEN $2 AND $3
             ORDER BY seq",
        )
        .bind(conversation_id)
        .bind(start_seq)
        .bind(end_seq)
        .fetch_all(&self.pool)
        .await?;

        self.assemble(conversation_id, rows).await
    }

    async fn get(&self, conversation_id: &str, server_id: &str) -> Result<Option<Msg>, Error> {
        let rows = sqlx::query(
            "SELECT server_id, seq, send_id, msg_type, content_type, send_time,
                    content_size, inline_content, detail::text AS detail,
                    is_revoked, revoke_time, revoked_by
             FROM msg_meta
             WHERE conversation_id = $1 AND server_id = $2",
        )
        .bind(conversation_id)
        .bind(server_id)
        .fetch_all(&self.pool)
        .await?;

        let mut msgs = self.assemble(conversation_id, rows).await?;
        Ok(msgs.pop())
    }

    async fn revoke(
        &self,
        conversation_id: &str,
        seq: i64,
        revoked_by: &str,
        revoke_time: i64,
    ) -> Result<(), Error> {
        sqlx::query(
            "UPDATE msg_meta
             SET is_revoked = TRUE, revoked_by = $3, revoke_time = $4
             WHERE conversation_id = $1 AND seq = $2",
        )
        .bind(conversation_id)
        .bind(seq)
        .bind(revoked_by)
        .bind(revoke_time)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_by_server_id(
        &self,
        conversation_id: &str,
        server_ids: &[String],
    ) -> Result<u64, Error> {
        if server_ids.is_empty() {
            return Ok(0);
        }

        let result = sqlx::query(
            "DELETE FROM msg_meta WHERE conversation_id = $1 AND server_id = ANY($2)",
        )
        .bind(conversation_id)
        .bind(server_ids)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<u64, Error> {
        let result = sqlx::query("DELETE FROM msg_meta WHERE conversation_id = $1")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

impl PgMsgMetaRepo {
    /// 把元数据行组装成 `Msg`，并为外置正文批量回表
    async fn assemble(
        &self,
        conversation_id: &str,
        rows: Vec<sqlx::postgres::PgRow>,
    ) -> Result<Vec<Msg>, Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        // 需要回表取正文的 server_id
        let missing: Vec<String> = rows
            .iter()
            .filter(|r| {
                let size: i32 = r.try_get("content_size").unwrap_or(0);
                let inline: Option<Vec<u8>> = r.try_get("inline_content").unwrap_or(None);
                size > 0 && inline.is_none()
            })
            .filter_map(|r| r.try_get::<String, _>("server_id").ok())
            .collect();

        let contents = if missing.is_empty() {
            Vec::new()
        } else {
            sqlx::query("SELECT server_id, content FROM msg_content WHERE server_id = ANY($1)")
                .bind(&missing)
                .fetch_all(&self.pool)
                .await?
        };

        let content_map: std::collections::HashMap<String, Vec<u8>> = contents
            .into_iter()
            .filter_map(|r| {
                let id: String = r.try_get("server_id").ok()?;
                let c: Vec<u8> = r.try_get("content").ok()?;
                Some((id, c))
            })
            .collect();

        let group_id = conversation_id
            .strip_prefix("g:")
            .unwrap_or("")
            .to_string();

        let mut msgs = Vec::with_capacity(rows.len());
        for row in rows {
            let server_id: String = row.try_get("server_id")?;
            let inline: Option<Vec<u8>> = row.try_get("inline_content")?;
            let content = inline
                .or_else(|| content_map.get(&server_id).cloned())
                .unwrap_or_default();

            let detail: String = row.try_get("detail")?;
            let detail: serde_json::Value = serde_json::from_str(&detail).unwrap_or_default();

            // receiver_id 属于用户视角（单聊取对端），此处留空由上层补齐
            msgs.push(Msg {
                server_id,
                seq: row.try_get("seq")?,
                send_id: row.try_get("send_id")?,
                receiver_id: String::new(),
                msg_type: row.try_get("msg_type")?,
                content_type: row.try_get("content_type")?,
                send_time: row.try_get("send_time")?,
                content,
                group_id: group_id.clone(),
                platform: detail
                    .get("platform")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32,
                related_msg_id: detail
                    .get("related_msg_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                is_revoked: row.try_get("is_revoked")?,
                revoke_time: row.try_get("revoke_time")?,
                revoked_by: row.try_get("revoked_by")?,
                // 读扩散下不存在「发送者序列号」，保持 0
                send_seq: 0,
                // 已读状态由上层按用户游标填充
                is_read: false,
                ..Default::default()
            });
        }

        Ok(msgs)
    }
}

/// 只保存无法从其他服务取到的低频字段。
/// 刻意不存 avatar / nickname：从用户缓存取，避免昵称修改后历史消息显示旧值。
fn detail_json(msg: &Msg) -> String {
    serde_json::json!({
        "platform": msg.platform,
        "related_msg_id": msg.related_msg_id,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// 用户会话游标
// ---------------------------------------------------------------------------

/// 用户会话仓库（`user_conversation` 表）
#[derive(Debug)]
pub struct PgUserConversationRepo {
    pool: PgPool,
}

impl PgUserConversationRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserConversationRepo for PgUserConversationRepo {
    async fn ensure_member(&self, user_id: &str, conversation_id: &str) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO user_conversation (user_id, conversation_id) VALUES ($1, $2)
             ON CONFLICT (user_id, conversation_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn ensure_members(
        &self,
        user_ids: &[String],
        conversation_id: &str,
    ) -> Result<u64, Error> {
        if user_ids.is_empty() {
            return Ok(0);
        }

        // 单语句多行写入：群成员登记一次往返完成
        let result = sqlx::query(
            "INSERT INTO user_conversation (user_id, conversation_id)
             SELECT u, $2 FROM unnest($1::varchar[]) AS u
             ON CONFLICT (user_id, conversation_id) DO NOTHING",
        )
        .bind(user_ids)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn mark_read(
        &self,
        user_id: &str,
        conversation_id: &str,
        read_seq: i64,
    ) -> Result<(), Error> {
        // 只增不减：乱序到达的旧回执不会把游标往回拖
        sqlx::query(
            "INSERT INTO user_conversation (user_id, conversation_id, last_read_seq)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id, conversation_id)
             DO UPDATE SET
                 last_read_seq = GREATEST(user_conversation.last_read_seq, EXCLUDED.last_read_seq),
                 updated_at = NOW()",
        )
        .bind(user_id)
        .bind(conversation_id)
        .bind(read_seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_cursor(&self, user_id: &str, conversation_id: &str) -> Result<i64, Error> {
        let row = sqlx::query(
            "SELECT last_read_seq FROM user_conversation
             WHERE user_id = $1 AND conversation_id = $2",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(row.try_get::<i64, _>("last_read_seq")?),
            None => Ok(0),
        }
    }

    async fn list(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<UserConversationState>, Error> {
        let rows = sqlx::query(
            "SELECT uc.conversation_id, uc.last_read_seq, uc.is_pinned, uc.is_muted
             FROM user_conversation uc
             JOIN conversation c ON c.conversation_id = uc.conversation_id
             WHERE uc.user_id = $1
             ORDER BY c.last_msg_time DESC, uc.conversation_id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                Ok(UserConversationState {
                    conversation_id: row.try_get("conversation_id")?,
                    last_read_seq: row.try_get("last_read_seq")?,
                    is_pinned: row.try_get("is_pinned")?,
                    is_muted: row.try_get("is_muted")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    async fn list_by_ids(
        &self,
        user_id: &str,
        conversation_ids: &[String],
    ) -> Result<Vec<UserConversationState>, Error> {
        if conversation_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            "SELECT conversation_id, last_read_seq, is_pinned, is_muted
             FROM user_conversation
             WHERE user_id = $1 AND conversation_id = ANY($2)",
        )
        .bind(user_id)
        .bind(conversation_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                Ok(UserConversationState {
                    conversation_id: row.try_get("conversation_id")?,
                    last_read_seq: row.try_get("last_read_seq")?,
                    is_pinned: row.try_get("is_pinned")?,
                    is_muted: row.try_get("is_muted")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    async fn unread(&self, user_id: &str, conversation_id: &str) -> Result<i64, Error> {
        let row = sqlx::query(
            "SELECT GREATEST(c.last_msg_seq - COALESCE(uc.last_read_seq, 0), 0) AS unread
             FROM conversation c
             LEFT JOIN user_conversation uc
                    ON uc.conversation_id = c.conversation_id AND uc.user_id = $1
             WHERE c.conversation_id = $2",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(row.try_get::<i64, _>("unread")?),
            None => Ok(0),
        }
    }
}
