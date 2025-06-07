use async_trait::async_trait;
use sqlx::{PgPool, Row};

use common::error::Error;
use common::proto::message::Msg;

use crate::message::MsgStoreRepo;

/// PostgreSQL消息存储实现
/// 负责将消息持久化存储到PostgreSQL数据库中
#[derive(Debug)]
pub struct PostgresMessage {
    pool: PgPool,
}

impl PostgresMessage {
    /// 创建新的PostgreSQL消息存储实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MsgStoreRepo for PostgresMessage {
    /// 保存消息到PostgreSQL数据库
    /// 使用ON CONFLICT DO UPDATE更新消息状态
    async fn save_message(&self, message: Msg) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO messages
             (local_id, server_id, send_id, receiver_id, msg_type, content_type, content, 
              send_time, platform, is_revoked, revoke_time, revoked_by, related_msg_id,
              forward_comment, is_forwarded, is_reply)
             VALUES
             ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (local_id) DO UPDATE SET
             is_revoked = EXCLUDED.is_revoked,
             revoke_time = EXCLUDED.revoke_time,
             revoked_by = EXCLUDED.revoked_by,
             related_msg_id = EXCLUDED.related_msg_id,
             forward_comment = EXCLUDED.forward_comment,
             is_forwarded = EXCLUDED.is_forwarded,
             is_reply = EXCLUDED.is_reply,
             updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&message.local_id)
        .bind(&message.server_id)
        .bind(&message.send_id)
        .bind(&message.receiver_id)
        .bind(message.msg_type)
        .bind(message.content_type)
        .bind(&message.content)
        .bind(message.send_time)
        .bind(message.platform)
        .bind(message.is_revoked)
        .bind(message.revoke_time)
        .bind(&message.revoked_by)
        .bind(message.related_msg_id.as_deref())
        .bind(message.forward_comment.as_deref())
        .bind(message.is_forwarded)
        .bind(message.is_reply)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 撤回消息
    async fn revoke_message(&self, message_id: &str, user_id: &str) -> Result<(), Error> {
        sqlx::query(
            "UPDATE messages 
             SET is_revoked = TRUE,
                 revoke_time = EXTRACT(EPOCH FROM CURRENT_TIMESTAMP) * 1000,
                 revoked_by = $1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE local_id = $2 OR server_id = $2",
        )
        .bind(user_id)
        .bind(message_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 删除消息
    async fn delete_message(&self, message_id: &str) -> Result<(), Error> {
        sqlx::query(
            "DELETE FROM messages 
             WHERE local_id = $1 OR server_id = $1",
        )
        .bind(message_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 获取消息
    async fn get_message(&self, message_id: &str) -> Result<Option<Msg>, Error> {
        let row = sqlx::query(
            "SELECT local_id, server_id, send_id, receiver_id, msg_type, content_type, content, 
             send_time, platform, is_revoked, revoke_time, revoked_by, related_msg_id,
             forward_comment, is_forwarded, is_reply, create_time, seq, send_seq, is_read,
             group_id, avatar, nickname
             FROM messages 
             WHERE local_id = $1 OR server_id = $1",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;
        
        match row {
            Some(row) => {
                let msg = Msg {
                    local_id: row.get("local_id"),
                    server_id: row.get("server_id"),
                    send_id: row.get("send_id"),
                    receiver_id: row.get("receiver_id"),
                    msg_type: row.get("msg_type"),
                    content_type: row.get("content_type"),
                    content: row.get("content"),
                    send_time: row.get("send_time"),
                    platform: row.get("platform"),
                    is_revoked: row.get("is_revoked"),
                    revoke_time: row.get("revoke_time"),
                    revoked_by: row.get("revoked_by"),
                    related_msg_id: row.get("related_msg_id"),
                    forward_comment: row.get("forward_comment"),
                    is_forwarded: row.get("is_forwarded"),
                    is_reply: row.get("is_reply"),
                    create_time: row.get("create_time"),
                    seq: row.get("seq"),
                    send_seq: row.get("send_seq"),
                    is_read: row.get("is_read"),
                    group_id: row.get("group_id"),
                    avatar: row.get("avatar"),
                    nickname: row.get("nickname"),
                };
                Ok(Some(msg))
            }
            None => Ok(None),
        }
    }
}
