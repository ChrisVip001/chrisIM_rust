//! 序列号域实现：用户序号、发送序号、会话号段、群成员批量序号
//!
//! 序号分配依赖两段 Lua 脚本保证原子性（脚本加载见 [`super`]）。

use async_trait::async_trait;
use contract::error::Error;
use contract::proto::message::GroupMemSeq;

use super::*;
use redis::AsyncCommands;

#[async_trait]
impl SeqCache for RedisCache {
    /// 检查序列号是否已加载
    ///
    /// 序列号加载是系统初始化的重要步骤，确保序列号的连续性
    async fn check_seq_loaded(&self) -> Result<bool, Error> {
        let mut conn = self.get_connection().await?;

        // redis.get::<K,U>() K是键，U是返回值类型
        let need_load = conn.get::<_, Option<String>>(IS_LOADED).await;
        match need_load {
            Ok(Some(value)) if value == SEQ_NO_NEED_LOAD => Ok(false),
            _ => Ok(true),
        }
    }

    /// 标记序列号已加载完成
    ///
    /// 设置一个标志，表明序列号已经成功从持久存储加载到缓存
    async fn set_seq_loaded(&self) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        conn.set::<_, _, ()>(IS_LOADED, SEQ_NO_NEED_LOAD).await?;
        Ok(())
    }

    /// 设置用户的发送和接收序列号
    ///
    /// 批量设置多个用户的序列号信息
    async fn set_seq(&self, max_seq: &[(String, i64, i64)]) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        let mut pipe = redis::pipe();
        for (user_id, send_max_seq, rec_max_seq) in max_seq {
            let key = format!("send_seq:{}", user_id);
            pipe.hset(&key, CUR_SEQ_KEY, send_max_seq);
            pipe.hset(&key, MAX_SEQ_KEY, send_max_seq);
            let key = format!("seq:{}", user_id);
            pipe.hset(&key, CUR_SEQ_KEY, rec_max_seq);
            pipe.hset(&key, MAX_SEQ_KEY, rec_max_seq);
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 设置用户的发送序列号
    ///
    /// 批量设置多个用户的发送序列号
    async fn set_send_seq(&self, max_seq: &[(String, i64)]) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        let mut pipe = redis::pipe();
        for (user_id, max_seq) in max_seq {
            let key = format!("send_seq:{}", user_id);
            pipe.hset(&key, CUR_SEQ_KEY, max_seq);
            pipe.hset(&key, MAX_SEQ_KEY, max_seq);
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 获取用户的接收序列号
    async fn get_seq(&self, user_id: &str) -> Result<i64, Error> {
        // 生成键
        let key = format!("seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        let seq: i64 = conn.hget(&key, CUR_SEQ_KEY).await.unwrap_or_default();
        Ok(seq)
    }

    /// 获取用户的当前接收和发送序列号
    async fn get_cur_seq(&self, user_id: &str) -> Result<(i64, i64), Error> {
        // 生成键
        let key1 = format!("seq:{}", user_id);
        let key2 = format!("send_seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        // 使用管道一次性获取两个值，减少网络往返
        // 使用Option类型处理可能的nil值
        let (seq1, seq2): (Option<i64>, Option<i64>) = redis::pipe()
            .cmd("HGET")
            .arg(&key1)
            .arg(CUR_SEQ_KEY)
            .cmd("HGET")
            .arg(&key2)
            .arg(CUR_SEQ_KEY)
            .query_async(&mut conn)
            .await?;

        // 处理默认值，如果Redis中没有数据则返回0
        let seq1 = seq1.unwrap_or_default();
        let seq2 = seq2.unwrap_or_default();

        Ok((seq1, seq2))
    }

    /// 获取用户的发送序列号信息
    async fn get_send_seq(&self, user_id: &str) -> Result<(i64, i64), Error> {
        // 生成键
        let key = format!("send_seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        // 使用管道一次性获取两个值，减少网络往返
        let (cur_seq, max_seq): (Option<i64>, Option<i64>) = redis::pipe()
            .cmd("HGET")
            .arg(&key)
            .arg("cur_seq")
            .cmd("HGET")
            .arg(&key)
            .arg("max_seq")
            .query_async(&mut conn)
            .await?;

        // 处理默认值
        let cur_seq = cur_seq.unwrap_or_default();
        let max_seq = max_seq.unwrap_or_default();

        Ok((cur_seq, max_seq))
    }

    /// 增加用户的接收序列号
    ///
    /// 原子方式增加序列号，并在需要时更新最大序列号
    async fn increase_seq(&self, user_id: &str) -> Result<(i64, i64, bool), Error> {
        // 生成键
        let key = format!("seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        // 增加序列号
        let seq = redis::cmd(EVALSHA)
            .arg(&self.single_seq_exe_sha)
            .arg(1)
            .arg(&key)
            .arg(self.seq_step)
            .query_async(&mut conn)
            .await?;
        Ok(seq)
    }

    /// 增加用户的发送序列号
    ///
    /// 原子方式增加发送序列号，并在需要时更新最大序列号
    async fn incr_send_seq(&self, user_id: &str) -> Result<(i64, i64, bool), Error> {
        // 生成键
        let key = format!("send_seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        // 增加序列号
        let seq = redis::cmd(EVALSHA)
            .arg(&self.single_seq_exe_sha)
            .arg(1)
            .arg(&key)
            .arg(self.seq_step)
            .query_async(&mut conn)
            .await?;
        Ok(seq)
    }

    /// 会话号段是否已初始化
    async fn conversation_seq_exists(&self, conversation_id: &str) -> Result<bool, Error> {
        let key = format!("cseq:{}", conversation_id);
        let mut conn = self.get_connection().await?;
        let exists: i64 = redis::cmd("EXISTS")
            .arg(&key)
            .query_async(&mut conn)
            .await?;
        Ok(exists > 0)
    }

    /// 用持久化水位初始化会话号段
    ///
    /// 用 HSETNX 而不是 HSET：多个消费者同时首次访问同一会话时，
    /// 只让第一个写入生效，避免互相覆盖号段。
    async fn init_conversation_seq(
        &self,
        conversation_id: &str,
        base_seq: i64,
    ) -> Result<i64, Error> {
        let key = format!("cseq:{}", conversation_id);
        let reserved = base_seq + self.seq_step as i64;
        let mut conn = self.get_connection().await?;

        let _: i64 = redis::cmd("HSETNX")
            .arg(&key)
            .arg("cur_seq")
            .arg(base_seq)
            .query_async(&mut conn)
            .await?;

        let _: i64 = redis::cmd("HSETNX")
            .arg(&key)
            .arg("max_seq")
            .arg(reserved)
            .query_async(&mut conn)
            .await?;

        // 并发初始化时只有一个 HSETNX 生效，但两者算出的上界相同，返回值一致
        Ok(reserved)
    }

    /// 领取会话下一个序号
    async fn incr_conversation_seq(
        &self,
        conversation_id: &str,
    ) -> Result<(i64, i64, bool), Error> {
        let key = format!("cseq:{}", conversation_id);
        let mut conn = self.get_connection().await?;

        let seq = redis::cmd(EVALSHA)
            .arg(&self.single_seq_exe_sha)
            .arg(1)
            .arg(&key)
            .arg(self.seq_step)
            .query_async(&mut conn)
            .await?;

        Ok(seq)
    }

    /// 增加群组成员序列号
    ///
    /// 一次性为多个群组成员增加序列号
    async fn incr_group_seq(&self, mut members: Vec<String>) -> Result<Vec<GroupMemSeq>, Error> {
        let mut conn = self.get_connection().await?;

        let mut cmd = redis::cmd(EVALSHA);
        cmd.arg(&self.group_seq_exe_sha).arg(0).arg(self.seq_step);

        for member in members.iter() {
            cmd.arg(member);
        }

        let response: Vec<redis::Value> = cmd.query_async(&mut conn).await?;

        let mut seq = Vec::with_capacity(members.len());
        for item in response.into_iter() {
            if let redis::Value::Array(bulk_item) = item {
                if bulk_item.len() == 3 {
                    if let (
                        redis::Value::Int(cur_seq),
                        redis::Value::Int(max_seq),
                        redis::Value::Int(updated),
                    ) = (&bulk_item[0], &bulk_item[1], &bulk_item[2])
                    {
                        seq.push(GroupMemSeq::new(
                            members.remove(0),
                            *cur_seq,
                            *max_seq,
                            *updated != 0,
                        ));
                    }
                }
            }
        }
        Ok(seq)
    }
}
