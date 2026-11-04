/**
 * Redis缓存模块实现
 *
 * 本模块实现缓存领域接口（见 `crate::traits`），实现按域拆分到子模块：
 * - [`seq`]：序列号与号段分配（Lua 脚本保证原子性）
 * - [`session`]：平台令牌、注册验证码
 * - [`relation`]：好友、黑名单、群成员
 * - [`presence`]：在线状态、网关推送路由
 * - [`unread`]：会话未读计数
 *
 * 本文件只保留所有域共享的基础设施：连接池、Lua 脚本加载、键前缀常量。
 *
 * 实现采用异步编程模式，通过连接池和信号量机制提高并发性能。
 */
use crate::traits::SeqCache;
use async_trait::async_trait;
use runtime::config::AppConfig;
use contract::error::Error;
use redis::aio::MultiplexedConnection;
use redis::{Client, RedisError};
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

mod presence;
mod relation;
mod seq;
mod session;
mod unread;

/// 群组成员ID前缀
const GROUP_MEMBERS_ID_PREFIX: &str = "group_members_id";

/// 注册验证码的键
const REGISTER_CODE_KEY: &str = "register_code";

/// 注册验证码过期时间（秒）
const REGISTER_CODE_EXPIRE: i64 = 300;

/// 在线用户集合
const USER_ONLINE_SET: &str = "user_online_set";

/// 用户平台在线状态前缀
const USER_PLATFORM_ONLINE_PREFIX: &str = "user_platform_online";
/// 用户网关路由表前缀：field=平台，value=网关gRPC地址
const USER_GATEWAY_ROUTE_PREFIX: &str = "user_gateway_route";

/// 用户好友集合前缀
const USER_FRIENDS_SET_PREFIX: &str = "user_friends";

/// 用户黑名单集合前缀
const USER_BLACKLIST_PREFIX: &str = "user_blacklist";

/// 用户未读消息计数前缀
const UNREAD_COUNT_PREFIX: &str = "unread_count";

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
///
/// 五个领域 trait（Seq/Session/Relation/Presence/Unread）分别在各子模块中
/// 为本类型实现；本类型只承载共享基础设施。
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
    pub async fn new(client: Client) -> Self {
        let seq_step = DEFAULT_SEQ_STEP;
        let max_connections = DEFAULT_MAX_CONNECTIONS;
        let connection_semaphore = Arc::new(Semaphore::new(max_connections));

        // 初始化连接管理器
        let connection_manager = client.get_multiplexed_async_connection().await.unwrap();

        // 加载Lua脚本
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let single_seq_exe_sha = Self::single_script_load(&mut conn).await.unwrap();
        let group_seq_exe_sha = Self::group_script_load(&mut conn).await.unwrap();

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
    pub async fn from_config(config: &AppConfig) -> Self {
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
        let connection_manager = client.get_multiplexed_async_connection().await.unwrap();

        // 加载Lua脚本
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let single_seq_exe_sha = Self::single_script_load(&mut conn).await.unwrap();
        let group_seq_exe_sha = Self::group_script_load(&mut conn).await.unwrap();

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
    pub(crate) async fn single_script_load(
        conn: &mut MultiplexedConnection,
    ) -> Result<String, RedisError> {
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
    pub(crate) async fn group_script_load(
        conn: &mut MultiplexedConnection,
    ) -> Result<String, RedisError> {
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
    pub(crate) async fn get_connection(&self) -> Result<MultiplexedConnection, Error> {
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

/// 序列号号段的边界行为
///
/// 号段分配由 Redis 侧的 Lua 脚本完成，无法用内存实现替代，
/// 因此这组用例需要一个真实的 Redis 实例，通过 `IM_TEST_REDIS_URL` 指定：
///
/// ```text
/// docker run --rm -p 6379:6379 redis:7
/// IM_TEST_REDIS_URL=redis://127.0.0.1:6379 cargo test -p cache
/// ```
///
/// 未设置该变量时用例主动跳过，保证本地 `cargo test` 不依赖外部服务。
#[cfg(test)]
mod seq_tests {
    use super::*;

    fn redis_url() -> Option<String> {
        std::env::var("IM_TEST_REDIS_URL")
            .ok()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
    }

    /// 用指定步长构造 RedisCache，便于在小步长下快速验证号段翻滚
    async fn cache_with_step(url: &str, step: i32) -> RedisCache {
        let client = redis::Client::open(url).expect("IM_TEST_REDIS_URL 不是合法的 Redis URL");

        let mut script_conn = client
            .get_multiplexed_async_connection()
            .await
            .expect("连接测试 Redis 失败");
        let single_seq_exe_sha = RedisCache::single_script_load(&mut script_conn)
            .await
            .expect("加载单序列号脚本失败");
        let group_seq_exe_sha = RedisCache::group_script_load(&mut script_conn)
            .await
            .expect("加载群组序列号脚本失败");

        let connection_manager = client
            .get_multiplexed_async_connection()
            .await
            .expect("连接测试 Redis 失败");

        RedisCache {
            client,
            connection_manager: Mutex::new(connection_manager),
            connection_semaphore: Arc::new(Semaphore::new(4)),
            seq_step: step,
            single_seq_exe_sha,
            group_seq_exe_sha,
            max_connections: 4,
        }
    }

    async fn reset_key(cache: &RedisCache, key: &str) {
        let mut conn = cache
            .client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let _: () = redis::cmd("DEL")
            .arg(key)
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn segment_rolls_over_exactly_at_step_boundary() {
        let Some(url) = redis_url() else {
            eprintln!("跳过 segment_rolls_over_exactly_at_step_boundary：未设置 IM_TEST_REDIS_URL");
            return;
        };

        let step = 3;
        let cache = cache_with_step(&url, step).await;
        let user_id = "seq-boundary-test";
        reset_key(&cache, &format!("seq:{}", user_id)).await;

        let mut observed = Vec::new();
        for _ in 0..10 {
            observed.push(cache.increase_seq(user_id).await.unwrap());
        }

        // 序列号从 1 开始严格连续递增
        for (index, (cur, _, _)) in observed.iter().enumerate() {
            assert_eq!(
                *cur,
                index as i64 + 1,
                "序列号必须连续，不能重号或跳号"
            );
        }

        // 预留上界每次翻滚前进一个步长
        assert_eq!(observed[0].1, 3, "首次分配即预留 1 到 step");
        assert_eq!(observed[3].1, 6);
        assert_eq!(observed[6].1, 9);
        assert_eq!(observed[9].1, 12);

        // updated 只在跨入新号段的那一条上为 true，这正是回写数据库水位的时机
        let rollovers: Vec<i64> = observed
            .iter()
            .filter(|(_, _, updated)| *updated)
            .map(|(cur, _, _)| *cur)
            .collect();
        assert_eq!(rollovers, vec![4, 7, 10], "步长为 3 时第 4/7/10 条触发翻滚");
    }

    #[tokio::test]
    async fn concurrent_allocation_never_duplicates() {
        let Some(url) = redis_url() else {
            eprintln!("跳过 concurrent_allocation_never_duplicates：未设置 IM_TEST_REDIS_URL");
            return;
        };

        let cache = Arc::new(cache_with_step(&url, 100).await);
        let user_id = "seq-concurrent-test";
        reset_key(&cache, &format!("seq:{}", user_id)).await;

        let mut handles = Vec::new();
        for _ in 0..50 {
            let cache = cache.clone();
            handles.push(tokio::spawn(async move {
                cache.increase_seq(user_id).await.unwrap().0
            }));
        }

        let mut seqs = Vec::with_capacity(handles.len());
        for handle in handles {
            seqs.push(handle.await.unwrap());
        }

        let total = seqs.len();
        seqs.sort_unstable();
        seqs.dedup();
        assert_eq!(seqs.len(), total, "并发分配不允许出现重复序列号");
    }
}
