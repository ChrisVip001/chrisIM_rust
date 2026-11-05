//! 会话列表索引：每个用户一个 ZSET，member 为会话 ID，score 为最近活跃时间
//!
//! 写入用 `ZADD ... GT`，只抬高分数，重复投递不会把会话往后挪；
//! 读取用 `ZREVRANGE ... WITHSCORES` 一次拿到「最近活跃的 N 个会话」。
//! 索引键带过期时间，长期不活跃的用户自动回落数据库查询并重建。

use async_trait::async_trait;
use contract::error::Error;
use std::collections::HashMap;

use super::*;
use crate::traits::ConversationIndexCache;

/// 会话列表索引前缀
const CONVERSATION_INDEX_PREFIX: &str = "conv_list";

/// 索引键过期时间（秒）：30 天不活跃后交给数据库重建
const CONVERSATION_INDEX_TTL: i64 = 30 * 24 * 60 * 60;

#[async_trait]
impl ConversationIndexCache for RedisCache {
    async fn touch_conversation_index(
        &self,
        entries: &[(String, String, i64)],
    ) -> Result<(), Error> {
        if entries.is_empty() {
            return Ok(());
        }

        // 同一个用户的多条记录合并成一次 ZADD，减少命令数
        let mut by_user: HashMap<&str, HashMap<&str, i64>> = HashMap::new();
        for (user_id, conversation_id, active_time) in entries {
            let slots = by_user.entry(user_id.as_str()).or_default();
            let slot = slots
                .entry(conversation_id.as_str())
                .or_insert(*active_time);
            if *active_time > *slot {
                *slot = *active_time;
            }
        }

        let mut conn = self.get_connection().await?;
        let mut pipe = redis::pipe();
        for (user_id, conversations) in by_user {
            let key = format!("{}:{}", CONVERSATION_INDEX_PREFIX, user_id);
            let mut cmd = redis::cmd("ZADD");
            cmd.arg(&key).arg("GT");
            for (conversation_id, active_time) in conversations {
                cmd.arg(active_time).arg(conversation_id);
            }
            pipe.add_command(cmd);
            pipe.expire(&key, CONVERSATION_INDEX_TTL);
        }

        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    async fn recent_conversations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, Error> {
        if limit <= 0 {
            return Ok(Vec::new());
        }

        let key = format!("{}:{}", CONVERSATION_INDEX_PREFIX, user_id);
        let mut conn = self.get_connection().await?;

        let flat: Vec<String> = redis::cmd("ZREVRANGE")
            .arg(&key)
            .arg(0)
            .arg(limit - 1)
            .arg("WITHSCORES")
            .query_async(&mut conn)
            .await?;

        Ok(parse_with_scores(flat))
    }
}

/// 解析 `ZREVRANGE ... WITHSCORES` 的扁平返回值
///
/// 返回值是 member/score 交替出现的字符串数组，分数以浮点字符串给出。
fn parse_with_scores(flat: Vec<String>) -> Vec<(String, i64)> {
    flat.chunks_exact(2)
        .map(|pair| {
            let active_time = pair[1].parse::<f64>().unwrap_or(0.0) as i64;
            (pair[0].clone(), active_time)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_with_scores;

    #[test]
    fn parses_members_with_scores() {
        let flat = vec![
            "c1".to_string(),
            "1700000000000".to_string(),
            "c2".to_string(),
            "1699999999000".to_string(),
        ];
        let parsed = parse_with_scores(flat);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], ("c1".to_string(), 1700000000000));
        assert_eq!(parsed[1], ("c2".to_string(), 1699999999000));
    }

    #[test]
    fn ignores_incomplete_tail() {
        // 缺分数时丢掉这一项，而不是 panic 或错位
        let parsed = parse_with_scores(vec!["c1".to_string()]);
        assert!(parsed.is_empty());
    }
}
