use crate::Error;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use once_cell::sync::Lazy;
use std::thread;

// 全局雪花ID生成器
pub static SNOWFLAKE: Lazy<Snowflake> = Lazy::new(|| {
    Snowflake::new(1, None).expect("Failed to initialize Snowflake generator")
});

/// Snowflake 算法配置
/// 默认结构：64位ID = 41位时间戳 + 10位节点ID + 12位序列号
#[derive(Debug, Clone)]
pub struct Snowflake {
    node_id: u64,          // 节点ID
    epoch: u64,            // 起始时间戳（毫秒）
    sequence: Arc<Mutex<u64>>, // 序列号（使用互斥锁保证线程安全）
    last_timestamp: Arc<Mutex<u64>>, // 上次生成时间戳
    max_clock_backwards_ms: u64,  // 最大允许的时钟回拨毫秒数
}

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
            sequence: Arc::new(Mutex::new(0)),
            last_timestamp: Arc::new(Mutex::new(0)),
            max_clock_backwards_ms: 10, // 默认允许最多10毫秒的时钟回拨
        })
    }

    /// 生成唯一ID
    pub fn generate(&self) -> Result<u64, Error> {
        let mut retry_count = 0;
        let max_retries = 3;
        
        loop {
            match self.try_generate() {
                Ok(id) => return Ok(id),
                Err(e) => {
                    if retry_count >= max_retries {
                        return Err(e);
                    }
                    
                    // 如果是时钟回拨问题，等待一段时间后重试
                    if let Error::Internal(msg) = &e {
                        if msg.contains("Clock moved backwards") {
                            thread::sleep(Duration::from_millis(5)); // 等待5毫秒
                            retry_count += 1;
                            continue;
                        }
                    }
                    
                    // 其他错误直接返回
                    return Err(e);
                }
            }
        }
    }
    
    /// 尝试生成ID的内部方法
    fn try_generate(&self) -> Result<u64, Error> {
        let mut timestamp = self.current_time()?;
        let mut sequence = self.sequence.lock().map_err(|e| 
            Error::Internal(format!("Failed to lock sequence: {}", e)))?;
        let mut last_timestamp = self.last_timestamp.lock().map_err(|e| 
            Error::Internal(format!("Failed to lock last_timestamp: {}", e)))?;

        // 时钟回拨检查
        if timestamp < *last_timestamp {
            let backwards_ms = *last_timestamp - timestamp;
            
            // 如果时钟回拨在允许范围内，则使用上次时间戳
            if backwards_ms <= self.max_clock_backwards_ms {
                timestamp = *last_timestamp;
            } else {
                return Err(Error::Internal(format!(
                    "Clock moved backwards by {} milliseconds", 
                    backwards_ms
                )));
            }
        }

        // 同一毫秒内递增序列号
        if timestamp == *last_timestamp {
            *sequence += 1;
            if *sequence >= 4096 {
                // 等待下一毫秒
                timestamp = self.wait_next_millis(*last_timestamp)?;
                *sequence = 0;
            }
        } else {
            *sequence = 0;
        }

        *last_timestamp = timestamp;

        // 组合ID
        let id = (timestamp - self.epoch) << 22
            | (self.node_id << 12)
            | *sequence;

        Ok(id)
    }

    /// 获取当前时间戳（毫秒）
    fn current_time(&self) -> Result<u64, Error> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Internal("Clock error".to_string()))
            .map(|d| d.as_millis() as u64)
    }

    /// 等待下一毫秒
    fn wait_next_millis(&self, last_timestamp: u64) -> Result<u64, Error> {
        let mut timestamp = self.current_time()?;
        let mut attempts = 0;
        let max_attempts = 100; // 防止无限循环
        
        while timestamp <= last_timestamp {
            // 防止过度自旋消耗CPU
            if attempts > 10 {
                thread::sleep(Duration::from_micros(50));
            }
            
            timestamp = self.current_time()?;
            attempts += 1;
            
            if attempts > max_attempts {
                return Err(Error::Internal(format!(
                    "Failed to get next millisecond after {} attempts", 
                    max_attempts
                )));
            }
        }
        
        Ok(timestamp)
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
} 