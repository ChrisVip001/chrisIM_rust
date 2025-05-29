use async_trait::async_trait;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

use common::error::Error;
use common::message::Msg;
use common::message_ext::MsgExt;

use crate::message::MsgStoreRepo;

#[derive(Debug)]
pub struct PostgresMessage {
    pool: PgPool,
}

/// 查询参数
#[derive(Debug, Clone)]
pub struct QueryParams {
    /// 用户ID
    pub user_id: String,
    /// 会话ID（单聊时是对方用户ID，群聊时是群组ID）
    pub conversation_id: Option<String>,
    /// 是否为群聊
    pub is_group: Option<bool>,
    /// 开始时间（毫秒时间戳）
    pub start_time: Option<i64>,
    /// 结束时间（毫秒时间戳）
    pub end_time: Option<i64>,
    /// 页面大小
    pub limit: Option<i64>,
    /// 偏移量
    pub offset: Option<i64>,
    /// 是否只查询未读消息
    pub unread_only: Option<bool>,
    /// 消息类型过滤
    pub msg_types: Option<Vec<i32>>,
    /// 内容类型过滤
    pub content_types: Option<Vec<i32>>,
}

/// 消息统计信息
#[derive(Debug, Clone)]
pub struct MessageStats {
    /// 总消息数
    pub total_count: i64,
    /// 未读消息数
    pub unread_count: i64,
    /// 单聊消息数
    pub private_count: i64,
    /// 群聊消息数
    pub group_count: i64,
    /// 最后一条消息时间
    pub last_message_time: Option<i64>,
}

/// 会话信息
#[derive(Debug, Clone)]
pub struct ConversationInfo {
    /// 会话ID
    pub conversation_id: String,
    /// 是否为群聊
    pub is_group: bool,
    /// 最后一条消息
    pub last_message: Option<Msg>,
    /// 未读消息数
    pub unread_count: i64,
    /// 最后更新时间
    pub last_update_time: i64,
}

impl PostgresMessage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 查询用户的消息列表
    /// 
    /// 使用优化的索引进行高效查询
    pub async fn query_messages(&self, params: QueryParams) -> Result<Vec<Msg>, Error> {
        let mut query_builder = sqlx::QueryBuilder::new("");
        
        // 根据是否为群聊选择不同的查询策略
        if let Some(is_group) = params.is_group {
            if is_group {
                // 群聊消息查询 - 使用群聊专用索引
                query_builder.push("SELECT * FROM messages WHERE group_id IS NOT NULL");
                
                if let Some(conversation_id) = &params.conversation_id {
                    query_builder.push(" AND group_id = ");
                    query_builder.push_bind(conversation_id);
                }
                
                query_builder.push(" AND receiver_id = ");
                query_builder.push_bind(&params.user_id);
            } else {
                // 单聊消息查询 - 使用单聊专用索引
                query_builder.push("SELECT * FROM messages WHERE group_id IS NULL");
                
                if let Some(conversation_id) = &params.conversation_id {
                    query_builder.push(" AND (");
                    query_builder.push("(receiver_id = ");
                    query_builder.push_bind(&params.user_id);
                    query_builder.push(" AND send_id = ");
                    query_builder.push_bind(conversation_id);
                    query_builder.push(") OR (receiver_id = ");
                    query_builder.push_bind(conversation_id);
                    query_builder.push(" AND send_id = ");
                    query_builder.push_bind(&params.user_id);
                    query_builder.push("))");
                } else {
                    query_builder.push(" AND (receiver_id = ");
                    query_builder.push_bind(&params.user_id);
                    query_builder.push(" OR send_id = ");
                    query_builder.push_bind(&params.user_id);
                    query_builder.push(")");
                }
            }
        } else {
            // 查询所有消息
            query_builder.push("SELECT * FROM messages WHERE (receiver_id = ");
            query_builder.push_bind(&params.user_id);
            query_builder.push(" OR send_id = ");
            query_builder.push_bind(&params.user_id);
            query_builder.push(")");
        }

        // 添加时间范围过滤
        if let Some(start_time) = params.start_time {
            query_builder.push(" AND send_time >= ");
            query_builder.push_bind(start_time);
        }
        
        if let Some(end_time) = params.end_time {
            query_builder.push(" AND send_time <= ");
            query_builder.push_bind(end_time);
        }

        // 添加未读消息过滤
        if let Some(true) = params.unread_only {
            query_builder.push(" AND is_read = false");
        }

        // 添加消息类型过滤
        if let Some(msg_types) = &params.msg_types {
            if !msg_types.is_empty() {
                query_builder.push(" AND msg_type = ANY(");
                query_builder.push_bind(msg_types);
                query_builder.push(")");
            }
        }

        // 添加内容类型过滤
        if let Some(content_types) = &params.content_types {
            if !content_types.is_empty() {
                query_builder.push(" AND content_type = ANY(");
                query_builder.push_bind(content_types);
                query_builder.push(")");
            }
        }

        // 排序和分页
        query_builder.push(" ORDER BY send_time DESC");
        
        if let Some(limit) = params.limit {
            query_builder.push(" LIMIT ");
            query_builder.push_bind(limit);
        }
        
        if let Some(offset) = params.offset {
            query_builder.push(" OFFSET ");
            query_builder.push_bind(offset);
        }

        let query = query_builder.build();
        let rows = query.fetch_all(&self.pool).await?;
        
        let messages = rows.into_iter().map(|row| self.row_to_msg(row)).collect();
        Ok(messages)
    }

    /// 查询用户的消息统计信息
    /// 
    /// 使用预计算的统计视图
    pub async fn get_message_stats(&self, user_id: &str) -> Result<MessageStats, Error> {
        let row = sqlx::query(
            "SELECT total_messages, unread_count, private_count, group_count, last_message_time 
             FROM user_message_stats WHERE user_id = $1"
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            Ok(MessageStats {
                total_count: row.try_get::<Option<i64>, _>("total_messages")?.unwrap_or(0),
                unread_count: row.try_get::<Option<i64>, _>("unread_count")?.unwrap_or(0),
                private_count: row.try_get::<Option<i64>, _>("private_count")?.unwrap_or(0),
                group_count: row.try_get::<Option<i64>, _>("group_count")?.unwrap_or(0),
                last_message_time: row.try_get("last_message_time")?,
            })
        } else {
            Ok(MessageStats {
                total_count: 0,
                unread_count: 0,
                private_count: 0,
                group_count: 0,
                last_message_time: None,
            })
        }
    }

    /// 查询用户的会话列表
    /// 
    /// 返回每个会话的最后一条消息和未读数量
    pub async fn get_conversations(&self, user_id: &str) -> Result<Vec<ConversationInfo>, Error> {
        let rows = sqlx::query(
            r#"
            WITH conversation_messages AS (
                SELECT 
                    CASE 
                        WHEN group_id IS NOT NULL AND group_id != '' THEN group_id
                        WHEN send_id = $1 THEN receiver_id
                        ELSE send_id
                    END as conversation_id,
                    group_id IS NOT NULL AND group_id != '' as is_group,
                    send_id,
                    receiver_id,
                    group_id,
                    local_id,
                    server_id,
                    create_time,
                    send_time,
                    seq,
                    send_seq,
                    msg_type,
                    content_type,
                    content,
                    is_read,
                    platform,
                    avatar,
                    nickname,
                    related_msg_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY 
                            CASE 
                                WHEN group_id IS NOT NULL AND group_id != '' THEN group_id
                                WHEN send_id = $1 THEN receiver_id
                                ELSE send_id
                            END
                        ORDER BY send_time DESC
                    ) as rn
                FROM messages 
                WHERE receiver_id = $1 OR send_id = $1
            ),
            conversation_stats AS (
                SELECT 
                    CASE 
                        WHEN group_id IS NOT NULL AND group_id != '' THEN group_id
                        WHEN send_id = $1 THEN receiver_id
                        ELSE send_id
                    END as conversation_id,
                    COUNT(*) FILTER (WHERE is_read = false AND receiver_id = $1) as unread_count
                FROM messages 
                WHERE receiver_id = $1 OR send_id = $1
                GROUP BY 
                    CASE 
                        WHEN group_id IS NOT NULL AND group_id != '' THEN group_id
                        WHEN send_id = $1 THEN receiver_id
                        ELSE send_id
                    END
            )
            SELECT 
                cm.conversation_id,
                cm.is_group,
                cm.send_id,
                cm.receiver_id,
                cm.group_id,
                cm.local_id,
                cm.server_id,
                cm.create_time,
                cm.send_time,
                cm.seq,
                cm.send_seq,
                cm.msg_type,
                cm.content_type,
                cm.content,
                cm.is_read,
                cm.platform,
                cm.avatar,
                cm.nickname,
                cm.related_msg_id,
                COALESCE(cs.unread_count, 0) as unread_count
            FROM conversation_messages cm
            LEFT JOIN conversation_stats cs ON cm.conversation_id = cs.conversation_id
            WHERE cm.rn = 1
            ORDER BY cm.send_time DESC
            "#
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        let conversations = rows.into_iter().map(|row| {
            let last_message = Msg {
                send_id: row.get("send_id"),
                receiver_id: row.get("receiver_id"),
                group_id: row.get::<Option<String>, _>("group_id").unwrap_or_default(),
                local_id: row.get("local_id"),
                server_id: row.get("server_id"),
                create_time: row.get("create_time"),
                send_time: row.get("send_time"),
                seq: row.get("seq"),
                send_seq: row.get("send_seq"),
                msg_type: row.get("msg_type"),
                content_type: row.get("content_type"),
                content: row.get("content"),
                is_read: row.get("is_read"),
                platform: row.get("platform"),
                avatar: row.get("avatar"),
                nickname: row.get("nickname"),
                related_msg_id: row.get("related_msg_id"),
            };

            ConversationInfo {
                conversation_id: row.get("conversation_id"),
                is_group: row.get("is_group"),
                last_message: Some(last_message),
                unread_count: row.get::<Option<i64>, _>("unread_count").unwrap_or(0),
                last_update_time: row.get("send_time"),
            }
        }).collect();

        Ok(conversations)
    }

    /// 标记消息为已读
    /// 
    /// 使用批量更新优化性能
    pub async fn mark_messages_as_read(
        &self, 
        user_id: &str, 
        conversation_id: &str, 
        is_group: bool,
        up_to_time: Option<i64>
    ) -> Result<u64, Error> {
        let mut query_builder = sqlx::QueryBuilder::new("UPDATE messages SET is_read = true WHERE receiver_id = ");
        query_builder.push_bind(user_id);
        query_builder.push(" AND is_read = false");

        if is_group {
            query_builder.push(" AND group_id = ");
            query_builder.push_bind(conversation_id);
        } else {
            query_builder.push(" AND group_id IS NULL AND send_id = ");
            query_builder.push_bind(conversation_id);
        }

        if let Some(time) = up_to_time {
            query_builder.push(" AND send_time <= ");
            query_builder.push_bind(time);
        }

        let result = query_builder.build().execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    /// 搜索消息内容
    /// 
    /// 使用全文搜索功能
    pub async fn search_messages(
        &self, 
        user_id: &str, 
        keyword: &str, 
        limit: Option<i64>
    ) -> Result<Vec<Msg>, Error> {
        let limit = limit.unwrap_or(50);
        
        let rows = sqlx::query(
            r#"
            SELECT send_id, receiver_id, group_id, local_id, server_id, create_time, send_time,
                   seq, send_seq, msg_type, content_type, content, is_read, platform,
                   avatar, nickname, related_msg_id
            FROM messages 
            WHERE (receiver_id = $1 OR send_id = $1)
              AND content_type = 1  -- 只搜索文本消息
              AND convert_from(content, 'UTF8') ILIKE $2
            ORDER BY send_time DESC
            LIMIT $3
            "#
        )
        .bind(user_id)
        .bind(format!("%{}%", keyword))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let messages = rows.into_iter().map(|row| Msg {
            send_id: row.get("send_id"),
            receiver_id: row.get("receiver_id"),
            group_id: row.get::<Option<String>, _>("group_id").unwrap_or_default(),
            local_id: row.get("local_id"),
            server_id: row.get("server_id"),
            create_time: row.get("create_time"),
            send_time: row.get("send_time"),
            seq: row.get("seq"),
            send_seq: row.get("send_seq"),
            msg_type: row.get("msg_type"),
            content_type: row.get("content_type"),
            content: row.get("content"),
            is_read: row.get("is_read"),
            platform: row.get("platform"),
            avatar: row.get("avatar"),
            nickname: row.get("nickname"),
            related_msg_id: row.get("related_msg_id"),
        }).collect();

        Ok(messages)
    }

    /// 获取未读消息数量（按会话分组）
    pub async fn get_unread_counts(&self, user_id: &str) -> Result<HashMap<String, i64>, Error> {
        let rows = sqlx::query(
            r#"
            SELECT 
                CASE 
                    WHEN group_id IS NOT NULL AND group_id != '' THEN group_id
                    ELSE send_id
                END as conversation_id,
                COUNT(*) as unread_count
            FROM messages 
            WHERE receiver_id = $1 AND is_read = false
            GROUP BY 
                CASE 
                    WHEN group_id IS NOT NULL AND group_id != '' THEN group_id
                    ELSE send_id
                END
            "#
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        let mut counts = HashMap::new();
        for row in rows {
            let conversation_id: String = row.get("conversation_id");
            let unread_count: Option<i64> = row.get("unread_count");
            counts.insert(conversation_id, unread_count.unwrap_or(0));
        }

        Ok(counts)
    }

    /// 删除消息
    /// 
    /// 支持软删除和硬删除
    pub async fn delete_messages(
        &self, 
        user_id: &str, 
        message_ids: &[String],
        hard_delete: bool
    ) -> Result<u64, Error> {
        if hard_delete {
            // 硬删除：直接从数据库中删除
            let result = sqlx::query(
                "DELETE FROM messages WHERE server_id = ANY($1) AND (send_id = $2 OR receiver_id = $2)"
            )
            .bind(message_ids)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            
            Ok(result.rows_affected())
        } else {
            // 软删除：标记为已删除
            let result = sqlx::query(
                "UPDATE messages SET content = '', content_type = 9 WHERE server_id = ANY($1) AND (send_id = $2 OR receiver_id = $2)"
            )
            .bind(message_ids)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            
            Ok(result.rows_affected())
        }
    }

    /// 将数据库行转换为消息对象
    fn row_to_msg(&self, row: sqlx::postgres::PgRow) -> Msg {
        Msg {
            send_id: row.get("send_id"),
            receiver_id: row.get("receiver_id"),
            group_id: row.get::<Option<String>, _>("group_id").unwrap_or_default(),
            local_id: row.get("local_id"),
            server_id: row.get("server_id"),
            create_time: row.get("create_time"),
            send_time: row.get("send_time"),
            seq: row.get("seq"),
            send_seq: row.get("send_seq"),
            msg_type: row.get("msg_type"),
            content_type: row.get("content_type"),
            content: row.get("content"),
            is_read: row.get("is_read"),
            platform: row.get("platform"),
            avatar: row.get("avatar"),
            nickname: row.get("nickname"),
            related_msg_id: row.get("related_msg_id"),
        }
    }
}

#[async_trait]
impl MsgStoreRepo for PostgresMessage {
    async fn save_message(&self, message: Msg) -> Result<(), Error> {
        // 使用优化后的表结构保存消息
        sqlx::query(
            r#"INSERT INTO messages
             (send_id, receiver_id, group_id, local_id, server_id, create_time, send_time, 
              seq, send_seq, msg_type, content_type, content, is_read, platform, avatar, nickname, related_msg_id)
             VALUES
             ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
             ON CONFLICT (server_id) DO NOTHING"#,
        )
        .bind(&message.send_id)
        .bind(&message.receiver_id)
        .bind(if message.group_id.is_empty() { None } else { Some(&message.group_id) })
        .bind(&message.local_id)
        .bind(&message.server_id)
        .bind(message.create_time)
        .bind(message.send_time)
        .bind(message.seq)
        .bind(message.send_seq)
        .bind(message.msg_type)
        .bind(message.content_type)
        .bind(&message.content)
        .bind(message.is_read)
        .bind(message.platform)
        .bind(&message.avatar)
        .bind(&message.nickname)
        .bind(message.related_msg_id.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
