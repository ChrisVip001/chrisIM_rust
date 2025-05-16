/**
 * Redis缓存模块实现
 *
 * 本模块实现了基于Redis的缓存功能，主要包括：
 * 1. 序列号管理 - 处理消息和通信的序列号生成与管理
 * 2. 群组成员管理 - 存储和检索群组成员信息
 * 3. 注册码管理 - 处理用户注册验证码的存储和验证
 * 4. 用户在线状态管理 - 跟踪用户的登录状态
 *
 * 该实现采用异步编程模式，通过连接池和信号量机制提高并发性能，
 * 同时使用Lua脚本进行原子操作，确保数据一致性。
 */
use crate::Cache;
use async_trait::async_trait;
use common::config::AppConfig;
use common::error::Error;
use common::message::GroupMemSeq;
use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, Client, RedisError};
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

/// 群组成员ID前缀
const GROUP_MEMBERS_ID_PREFIX: &str = "group_members_id";

/// 注册验证码的键
const REGISTER_CODE_KEY: &str = "register_code";

/// 注册验证码过期时间（秒）
const REGISTER_CODE_EXPIRE: i64 = 300;

/// 在线用户集合
const USER_ONLINE_SET: &str = "user_online_set";

/// 默认序列号步长
const DEFAULT_SEQ_STEP: i32 = 5000;

/// 执行Lua脚本的命令
const EVALSHA: &str = "EVALSHA";

/// 当前序列号的键
const CUR_SEQ_KEY: &str = "cur_seq";

/// 最大序列号的键
const MAX_SEQ_KEY: &str = "max_seq";

/// 序列号是否已加载的键
const IS_LOADED: &str = "seq_need_load";

/// 序列号不需要加载的值
const SEQ_NO_NEED_LOAD: &str = "false";

/// 默认最大连接数
const DEFAULT_MAX_CONNECTIONS: usize = 20;

/// Redis缓存实现
pub struct RedisCache {
    /// Redis客户端
    client: Client,
    /// 连接管理器，提供连接池功能
    connection_manager: Mutex<MultiplexedConnection>,
    /// 限制并发连接数的信号量
    connection_semaphore: Arc<Semaphore>,
    /// 序列号步长，每次增加序列号时的增量
    seq_step: i32,
    /// 单序列号生成Lua脚本的SHA值
    single_seq_exe_sha: String,
    /// 群组序列号生成Lua脚本的SHA值
    group_seq_exe_sha: String,
    /// 最大连接数
    max_connections: usize,
}

/// 为RedisCache实现Debug特征
impl Debug for RedisCache {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisCache")
            .field("client", &self.client)
            .field("connection_semaphore", &self.connection_semaphore)
            .field("seq_step", &self.seq_step)
            .field("single_seq_exe_sha", &self.single_seq_exe_sha)
            .field("group_seq_exe_sha", &self.group_seq_exe_sha)
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

impl RedisCache {
    /// 通过Redis客户端创建新的RedisCache实例
    ///
    /// 该方法会初始化连接管理器，设置默认参数，并加载Lua脚本
    ///
    /// # 参数
    /// * `client` - Redis客户端实例
    #[allow(dead_code)]
    pub fn new(client: Client) -> Self {
        let seq_step = DEFAULT_SEQ_STEP;
        let max_connections = DEFAULT_MAX_CONNECTIONS;
        let connection_semaphore = Arc::new(Semaphore::new(max_connections));

        // 初始化连接管理器
        let connection_manager = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { client.get_multiplexed_async_connection().await.unwrap() });

        // 加载Lua脚本
        let (single_seq_exe_sha, group_seq_exe_sha) =
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let mut conn = client.get_multiplexed_async_connection().await.unwrap();
                let single_sha = Self::single_script_load(&mut conn).await.unwrap();
                let group_sha = Self::group_script_load(&mut conn).await.unwrap();
                (single_sha, group_sha)
            });

        Self {
            client,
            connection_manager: Mutex::new(connection_manager),
            connection_semaphore,
            seq_step,
            single_seq_exe_sha,
            group_seq_exe_sha,
            max_connections,
        }
    }

    /// 从配置创建RedisCache实例
    ///
    /// 使用应用配置初始化Redis缓存，包括连接信息、最大连接数等参数
    ///
    /// # 参数
    /// * `config` - 应用配置对象
    pub fn from_config(config: &AppConfig) -> Self {
        // 使用unwrap是有意的，确保Redis连接在启动时就可用。
        // 如果无法连接Redis，程序应该崩溃，因为这对操作至关重要。
        let client = Client::open(config.redis.url()).unwrap();

        // 配置最大连接数，默认为20
        let max_connections = config
            .redis
            .max_connections
            .unwrap_or(DEFAULT_MAX_CONNECTIONS);
        let connection_semaphore = Arc::new(Semaphore::new(max_connections));

        // 初始化连接管理器
        let connection_manager = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { client.get_multiplexed_async_connection().await.unwrap() });

        // 加载Lua脚本
        let (single_seq_exe_sha, group_seq_exe_sha) =
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let mut conn = client.get_multiplexed_async_connection().await.unwrap();
                let single_sha = Self::single_script_load(&mut conn).await.unwrap();
                let group_sha = Self::group_script_load(&mut conn).await.unwrap();
                (single_sha, group_sha)
            });

        let mut seq_step = DEFAULT_SEQ_STEP;
        if config.redis.seq_step != 0 {
            seq_step = config.redis.seq_step;
        }

        Self {
            client,
            connection_manager: Mutex::new(connection_manager),
            connection_semaphore,
            seq_step,
            single_seq_exe_sha,
            group_seq_exe_sha,
            max_connections,
        }
    }

    /// 加载单序列号生成的Lua脚本
    ///
    /// 该脚本用于原子方式增加序列号并在需要时更新最大序列号
    ///
    /// # 参数
    /// * `conn` - Redis连接
    ///
    /// # 返回
    /// * 脚本的SHA值，用于后续执行
    async fn single_script_load(conn: &mut MultiplexedConnection) -> Result<String, RedisError> {
        let script = r#"
        local cur_seq = redis.call('HINCRBY', KEYS[1], 'cur_seq', 1)
        local max_seq = redis.call('HGET', KEYS[1], 'max_seq')
        local updated = false
        if max_seq == false then
            max_seq = tonumber(ARGV[1])
            redis.call('HSET', KEYS[1], 'max_seq', max_seq)
            end
        if tonumber(cur_seq) > tonumber(max_seq) then
            max_seq = tonumber(max_seq) + ARGV[1]
            redis.call('HSET', KEYS[1], 'max_seq', max_seq)
            updated = true
        end
        return {cur_seq, max_seq, updated}
        "#;
        redis::Script::new(script)
            .prepare_invoke()
            .load_async(conn)
            .await
    }

    /// 加载群组序列号生成的Lua脚本
    ///
    /// 该脚本用于批量处理多个成员的序列号更新
    ///
    /// # 参数
    /// * `conn` - Redis连接
    ///
    /// # 返回
    /// * 脚本的SHA值，用于后续执行
    async fn group_script_load(conn: &mut MultiplexedConnection) -> Result<String, RedisError> {
        let script = r#"
        local seq_step = tonumber(ARGV[1])
        local result = {}

        for i=2,#ARGV do
            local key = "seq:" .. ARGV[i]
            local cur_seq = redis.call('HINCRBY', key, 'cur_seq', 1)
            local max_seq = redis.call('HGET', key, 'max_seq')
            local updated = 0
            if max_seq == false then
                max_seq = seq_step
                redis.call('HSET', key, 'max_seq', max_seq)
            else
                max_seq = tonumber(max_seq)
            end
            if cur_seq > max_seq then
                max_seq = max_seq + seq_step
                redis.call('HSET', key, 'max_seq', max_seq)
                updated = 1
            end
            table.insert(result, {cur_seq, max_seq, updated})
        end

        return result
        "#;
        redis::Script::new(script)
            .prepare_invoke()
            .load_async(conn)
            .await
    }

    /// 获取连接的辅助方法，使用信号量限制并发连接数
    ///
    /// 通过信号量机制控制并发连接数，防止过载并确保资源合理分配
    ///
    /// # 返回
    /// * 成功则返回连接管理器实例
    /// * 失败则返回错误
    async fn get_connection(&self) -> Result<MultiplexedConnection, Error> {
        // 获取信号量许可，限制并发连接数
        let _permit = self.connection_semaphore.acquire().await.map_err(|e| {
            // 将信号量错误转换为内部错误
            Error::Internal(format!("获取连接信号量失败: {}", e))
        })?;

        // 从连接管理器获取连接
        let conn = self.connection_manager.lock().await;
        Ok(conn.clone())
    }
}

#[async_trait]
impl Cache for RedisCache {
    /// 检查序列号是否已加载
    ///
    /// 序列号加载是系统初始化的重要步骤，确保序列号的连续性
    ///
    /// # 返回
    /// * `true` - 序列号需要加载
    /// * `false` - 序列号已加载，不需要再次加载
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
        conn.set(IS_LOADED, SEQ_NO_NEED_LOAD).await?;
        Ok(())
    }

    /// 设置用户的发送和接收序列号
    ///
    /// 批量设置多个用户的序列号信息
    ///
    /// # 参数
    /// * `max_seq` - 包含用户ID、发送最大序列号和接收最大序列号的元组数组
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
        pipe.query_async(&mut conn).await?;
        Ok(())
    }

    /// 设置用户的发送序列号
    ///
    /// 批量设置多个用户的发送序列号
    ///
    /// # 参数
    /// * `max_seq` - 包含用户ID和发送最大序列号的元组数组
    async fn set_send_seq(&self, max_seq: &[(String, i64)]) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        let mut pipe = redis::pipe();
        for (user_id, max_seq) in max_seq {
            let key = format!("send_seq:{}", user_id);
            pipe.hset(&key, CUR_SEQ_KEY, max_seq);
            pipe.hset(&key, MAX_SEQ_KEY, max_seq);
        }
        pipe.query_async(&mut conn).await?;
        Ok(())
    }

    /// 获取用户的接收序列号
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    ///
    /// # 返回
    /// * 用户的当前接收序列号
    async fn get_seq(&self, user_id: &str) -> Result<i64, Error> {
        // 生成键
        let key = format!("seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        let seq: i64 = conn.hget(&key, CUR_SEQ_KEY).await.unwrap_or_default();
        Ok(seq)
    }

    /// 获取用户的当前接收和发送序列号
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    ///
    /// # 返回
    /// * 包含接收序列号和发送序列号的元组
    async fn get_cur_seq(&self, user_id: &str) -> Result<(i64, i64), Error> {
        // 生成键
        let key1 = format!("seq:{}", user_id);
        let key2 = format!("send_seq:{}", user_id);

        let mut conn = self.get_connection().await?;
        // 使用管道一次性获取两个值，减少网络往返
        let (seq1, seq2): (i64, i64) = redis::pipe()
            .cmd("HGET")
            .arg(&key1)
            .arg(CUR_SEQ_KEY)
            .cmd("HGET")
            .arg(&key2)
            .arg(CUR_SEQ_KEY)
            .query_async(&mut conn)
            .await?;

        Ok((seq1, seq2))
    }

    /// 获取用户的发送序列号信息
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    ///
    /// # 返回
    /// * 包含当前发送序列号和最大发送序列号的元组
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
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    ///
    /// # 返回
    /// * 包含当前序列号、最大序列号和是否更新的元组
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
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    ///
    /// # 返回
    /// * 包含当前序列号、最大序列号和是否更新的元组
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

    /// 增加群组成员序列号
    ///
    /// 一次性为多个群组成员增加序列号
    ///
    /// # 参数
    /// * `members` - 群组成员ID列表
    ///
    /// # 返回
    /// * 每个成员的序列号信息列表
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

    /// 查询群组成员ID列表
    ///
    /// Redis中的群组成员信息以集合形式存储，键为 group_members_id:group_id
    ///
    /// # 参数
    /// * `group_id` - 群组ID
    ///
    /// # 返回
    /// * 该群组的成员ID列表
    async fn query_group_members_id(&self, group_id: &str) -> Result<Vec<String>, Error> {
        // 生成键
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        // 从Redis查询值
        let mut conn = self.get_connection().await?;

        let result: Vec<String> = conn.smembers(&key).await?;
        Ok(result)
    }

    /// 保存群组成员ID列表
    ///
    /// 批量添加成员到群组，通常在创建群组时调用
    ///
    /// # 参数
    /// * `group_id` - 群组ID
    /// * `members_id` - 成员ID列表
    async fn save_group_members_id(
        &self,
        group_id: &str,
        members_id: Vec<String>,
    ) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        // 通过Redis管道为群组添加每个成员
        let mut pipe = redis::pipe();
        for member in members_id {
            pipe.sadd(&key, &member);
        }
        pipe.query_async(&mut conn).await?;
        Ok(())
    }

    /// 添加单个成员到群组
    ///
    /// # 参数
    /// * `member_id` - 成员ID
    /// * `group_id` - 群组ID
    async fn add_group_member_id(&self, member_id: &str, group_id: &str) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.sadd(&key, member_id).await?;
        Ok(())
    }

    /// 从群组移除单个成员
    ///
    /// # 参数
    /// * `group_id` - 群组ID
    /// * `member_id` - 要移除的成员ID
    async fn remove_group_member_id(&self, group_id: &str, member_id: &str) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.srem(&key, member_id).await?;
        Ok(())
    }

    /// 批量从群组移除成员
    ///
    /// # 参数
    /// * `group_id` - 群组ID
    /// * `member_id` - 要移除的成员ID数组
    async fn remove_group_member_batch(
        &self,
        group_id: &str,
        member_id: &[&str],
    ) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.srem(&key, member_id).await?;
        Ok(())
    }

    /// 删除群组的所有成员
    ///
    /// # 参数
    /// * `group_id` - 群组ID
    async fn del_group_members(&self, group_id: &str) -> Result<(), Error> {
        let key = format!("{}:{}", GROUP_MEMBERS_ID_PREFIX, group_id);
        let mut conn = self.get_connection().await?;
        conn.del(&key).await?;
        Ok(())
    }

    /// 保存用户注册验证码
    ///
    /// 将验证码与邮箱关联并设置5分钟过期时间
    ///
    /// # 参数
    /// * `email` - 用户邮箱
    /// * `code` - 验证码
    async fn save_register_code(&self, email: &str, code: &str) -> Result<(), Error> {
        // 设置注册码，有效期5分钟
        let mut conn = self.get_connection().await?;
        // 使用管道执行两个命令
        let mut pipe = redis::pipe();
        pipe.hset(REGISTER_CODE_KEY, email, code)
            .expire(REGISTER_CODE_KEY, REGISTER_CODE_EXPIRE)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// 获取用户注册验证码
    ///
    /// # 参数
    /// * `email` - 用户邮箱
    ///
    /// # 返回
    /// * 对应的验证码，如果不存在则返回None
    async fn get_register_code(&self, email: &str) -> Result<Option<String>, Error> {
        let mut conn = self.get_connection().await?;
        let result = conn.hget(REGISTER_CODE_KEY, email).await?;
        Ok(result)
    }

    /// 删除用户注册验证码
    ///
    /// 用户注册成功后删除验证码
    ///
    /// # 参数
    /// * `email` - 用户邮箱
    async fn del_register_code(&self, email: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        conn.hdel(REGISTER_CODE_KEY, email).await?;
        Ok(())
    }

    /// 用户登录
    ///
    /// 将用户ID添加到在线用户集合
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    async fn user_login(&self, user_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        conn.sadd(USER_ONLINE_SET, user_id).await?;
        Ok(())
    }

    /// 用户登出
    ///
    /// 从在线用户集合中移除用户ID
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    async fn user_logout(&self, user_id: &str) -> Result<(), Error> {
        let mut conn = self.get_connection().await?;
        conn.srem(USER_ONLINE_SET, user_id).await?;
        Ok(())
    }

    /// 获取在线用户数量
    ///
    /// # 返回
    /// * 当前在线用户数量
    async fn online_count(&self) -> Result<i64, Error> {
        let mut conn = self.get_connection().await?;
        let result: i64 = conn.scard(USER_ONLINE_SET).await?;
        Ok(result)
    }
}

/// 测试模块
#[cfg(test)]
mod tests {
    use super::*;
    use common::config::AppConfig;
    use std::ops::Deref;
    use std::thread;
    use tokio::runtime::Runtime;

    /// 测试辅助结构，管理Redis测试实例和自动清理
    struct TestRedis {
        client: redis::Client,
        cache: RedisCache,
    }

    impl Deref for TestRedis {
        type Target = RedisCache;
        fn deref(&self) -> &Self::Target {
            &self.cache
        }
    }

    /// 实现Drop特征，确保测试结束后清理Redis数据库
    impl Drop for TestRedis {
        fn drop(&mut self) {
            let client = self.client.clone();
            thread::spawn(move || {
                Runtime::new().unwrap().block_on(async {
                    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
                    // 使用let _: ()告诉编译器query_async方法的返回类型是()
                    let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
                })
            })
            .join()
            .unwrap();
        }
    }

    impl TestRedis {
        /// 创建一个新的测试Redis实例
        ///
        /// 默认使用数据库9进行测试
        fn new() -> Self {
            // 使用数据库9进行测试
            let database = 9;
            Self::from_db(database)
        }

        /// 从指定数据库创建测试Redis实例
        ///
        /// # 参数
        /// * `db` - 数据库编号
        fn from_db(db: u8) -> Self {
            let config = AppConfig::from_file(Some("./config/config.yaml")).unwrap();
            let url = format!("{}/{}", config.redis.url(), db);
            let client = redis::Client::open(url).unwrap();
            let cache = RedisCache::new(client.clone());
            TestRedis { client, cache }
        }
    }

    /// 测试增加序列号功能
    #[tokio::test]
    async fn test_increase_seq() {
        let user_id = "test";
        let cache = TestRedis::new();
        let seq = cache.increase_seq(user_id).await.unwrap();
        assert_eq!(seq, (1, DEFAULT_SEQ_STEP as i64, false));
    }

    /// 测试保存群组成员ID功能
    #[tokio::test]
    async fn test_save_group_members_id() {
        let group_id = "test";
        let members_id = vec!["1".to_string(), "2".to_string()];
        let cache = TestRedis::new();
        let result = cache.save_group_members_id(group_id, members_id).await;
        assert!(result.is_ok());
    }

    /// 测试查询群组成员ID功能
    #[tokio::test]
    async fn test_query_group_members_id() {
        let group_id = "test";
        let members_id = vec!["1".to_string(), "2".to_string()];
        let db = 8;
        let cache = TestRedis::from_db(db);
        let result = cache.save_group_members_id(group_id, members_id).await;
        assert!(result.is_ok());
        let result = cache.query_group_members_id(group_id).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"1".to_string()));
        assert!(result.contains(&"2".to_string()));
    }

    /// 测试添加群组成员功能
    #[tokio::test]
    async fn test_add_group_member_id() {
        let group_id = "test";
        let member_id = "1";
        let cache = TestRedis::new();
        let result = cache.add_group_member_id(member_id, group_id).await;
        assert!(result.is_ok());
    }

    /// 测试移除群组成员功能
    #[tokio::test]
    async fn test_remove_group_member_id() {
        let group_id = "test";
        let member_id = "1";
        let cache = TestRedis::new();
        let result = cache.add_group_member_id(member_id, group_id).await;
        assert!(result.is_ok());
        let result = cache.remove_group_member_id(group_id, member_id).await;
        assert!(result.is_ok());
    }

    /// 测试删除群组成员功能
    #[tokio::test]
    async fn test_del_group_members() {
        let group_id = "test";
        let members_id = vec!["1".to_string(), "2".to_string()];
        let cache = TestRedis::new();
        // 需要先添加成员
        let result = cache.save_group_members_id(group_id, members_id).await;
        assert!(result.is_ok());
        let result = cache.del_group_members(group_id).await;
        assert!(result.is_ok());
    }
}
