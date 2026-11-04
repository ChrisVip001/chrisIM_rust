use contract::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use once_cell::sync::Lazy;
use std::thread;

// 全局雪花ID生成器
pub static SNOWFLAKE: Lazy<Snowflake> = Lazy::new(|| {
    Snowflake::new(1, None).expect("Failed to initialize Snowflake generator")
});

/// 内部状态：时间戳与序号合并到单一临界区
///
/// 此前时间戳与序号分属两把锁，且「等待下一毫秒」的自旋/休眠
/// 发生在持锁期间，会阻塞其他生成者乃至异步 worker 线程。
#[derive(Debug, Default)]
struct State {
    last_timestamp: u64,
    sequence: u64,
}

/// 生成尝试的内部结果
enum TryOutcome {
    /// 成功生成
    Generated(u64),
    /// 当前毫秒序号耗尽，等待下一毫秒后重试
    NextMillis,
}

/// Snowflake 算法配置
/// 默认结构：64位ID = 41位时间戳 + 10位节点ID + 12位序列号
#[derive(Debug, Clone)]
pub struct Snowflake {
    node_id: u64,          // 节点ID
    epoch: u64,            // 起始时间戳（毫秒）
    state: Arc<Mutex<State>>, // 状态（临界区内只做算术，持锁时间微秒级）
    max_clock_backwards_ms: u64,  // 最大允许的时钟回拨毫秒数
}

/// 单毫秒内的序号上限（12位）
const SEQUENCE_BITS_MAX: u64 = 4096;

impl Snowflake {
    /// 创建新的 Snowflake 实例
    /// - node_id: 节点ID (0-1023)
    /// - epoch: 自定义起始时间（可选）
    pub fn new(node_id: u64, epoch: Option<u64>) -> Result<Self, Error> {
        if node_id >= 1024 {
            return Err(Error::Internal("Node ID must be between 0 and 1023".to_string()));
        }

        Ok(Self {
            node_id,
            epoch: epoch.unwrap_or(1_577_836_800_000), // 默认 2020-01-01
            state: Arc::new(Mutex::new(State::default())),
            max_clock_backwards_ms: 10, // 默认允许最多10毫秒的时钟回拨
        })
    }

    /// 生成唯一ID
    ///
    /// 可在异步上下文中安全调用：临界区内只有算术运算，
    /// 序号耗尽时的等待通过「锁外短暂休眠 + 重试」完成，
    /// 单次休眠 1ms，最多等待 100ms 后放弃。
    pub fn generate(&self) -> Result<u64, Error> {
        let max_wait_attempts = 100;
        for _ in 0..max_wait_attempts {
            match self.try_generate()? {
                TryOutcome::Generated(id) => return Ok(id),
                TryOutcome::NextMillis => {
                    // 锁外休眠，不阻塞其他生成者与异步 worker
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        Err(Error::Internal("Failed to generate snowflake id within wait budget".to_string()))
    }

    /// 尝试生成ID（锁内路径）
    fn try_generate(&self) -> Result<TryOutcome, Error> {
        let mut state = self.state.lock().map_err(|e| {
            Error::Internal(format!("Failed to lock snowflake state: {}", e))
        })?;

        let mut timestamp = self.current_time()?;

        // 时钟回拨检查
        if timestamp < state.last_timestamp {
            let backwards_ms = state.last_timestamp - timestamp;

            // 如果时钟回拨在允许范围内，则沿用上次时间戳
            if backwards_ms <= self.max_clock_backwards_ms {
                timestamp = state.last_timestamp;
            } else {
                return Err(Error::Internal(format!(
                    "Clock moved backwards by {} milliseconds",
                    backwards_ms
                )));
            }
        }

        // 同一毫秒内递增序列号
        if timestamp == state.last_timestamp {
            state.sequence += 1;
            if state.sequence >= SEQUENCE_BITS_MAX {
                // 当前毫秒序号耗尽：交还锁，等待下一毫秒后重试
                return Ok(TryOutcome::NextMillis);
            }
        } else {
            state.sequence = 0;
        }

        state.last_timestamp = timestamp;

        // 组合ID
        let id = (timestamp - self.epoch) << 22
            | (self.node_id << 12)
            | state.sequence;

        Ok(TryOutcome::Generated(id))
    }

    /// 获取当前时间戳（毫秒）
    fn current_time(&self) -> Result<u64, Error> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Internal("Clock error".to_string()))
            .map(|d| d.as_millis() as u64)
    }

    /// 设置最大允许的时钟回拨毫秒数
    pub fn set_max_clock_backwards_ms(&mut self, max_ms: u64) {
        self.max_clock_backwards_ms = max_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snowflake_id_generation() {
        let snowflake = Snowflake::new(1, None).unwrap();

        // 生成多个ID并验证唯一性
        let mut ids = Vec::new();
        for _ in 0..100 {
            let id = snowflake.generate().unwrap();
            assert!(!ids.contains(&id), "ID重复: {}", id);
            ids.push(id);
        }

        // 验证ID递增
        for i in 1..ids.len() {
            assert!(ids[i] > ids[i-1], "ID不是递增的: {} <= {}", ids[i], ids[i-1]);
        }
    }

    #[test]
    fn test_node_id_validation() {
        // 有效的节点ID
        assert!(Snowflake::new(0, None).is_ok());
        assert!(Snowflake::new(1023, None).is_ok());

        // 无效的节点ID
        assert!(Snowflake::new(1024, None).is_err());
    }

    #[test]
    fn test_concurrent_generation_is_unique() {
        let snowflake = Arc::new(Snowflake::new(2, None).unwrap());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let sf = snowflake.clone();
                std::thread::spawn(move || {
                    (0..500).map(|_| sf.generate().unwrap()).collect::<Vec<_>>()
                })
            })
            .collect();

        let mut all: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort_unstable();
        let before = all.len();
        all.dedup();
        assert_eq!(all.len(), before, "并发生成的ID存在重复");
    }
}
