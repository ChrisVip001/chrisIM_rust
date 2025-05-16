use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tonic::Status;
use tracing::{info, warn};

use super::ValidationResult;

// 缓存的验证结果
#[derive(Clone)]
struct CachedValidation {
    is_valid: bool,
    error: Option<String>,
    timestamp: Instant,
}

/// 通用验证中间件
/// 支持缓存、限流和事件跟踪
pub struct ValidationMiddleware {
    // 缓存
    cache: Arc<RwLock<HashMap<String, CachedValidation>>>,
    // 限流计数器
    rate_limits: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    // 配置
    cache_ttl: Duration,
    rate_limit_window: Duration,
    rate_limit_max_calls: usize,
}

impl ValidationMiddleware {
    /// 创建新的验证中间件
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(60), // 默认缓存1分钟
            rate_limit_window: Duration::from_secs(60), // 1分钟窗口
            rate_limit_max_calls: 100, // 每分钟最多100次调用
        }
    }
    
    /// 设置缓存TTL
    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }
    
    /// 设置限流参数
    pub fn with_rate_limit(mut self, window: Duration, max_calls: usize) -> Self {
        self.rate_limit_window = window;
        self.rate_limit_max_calls = max_calls;
        self
    }
    
    /// 缓存验证结果
    pub async fn cache_validation<F, T>(&self, key: &str, validation_fn: F) -> ValidationResult<T>
    where
        F: FnOnce() -> ValidationResult<T> + Send,
        T: Clone + Send + 'static,
    {
        // 生成缓存键
        let cache_key = format!("validation:{}", key);
        
        // 检查缓存
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&cache_key) {
                if cached.timestamp.elapsed() < self.cache_ttl {
                    if cached.is_valid {
                        // 缓存命中，有效结果
                        return Ok(T::default());
                    } else if let Some(error) = &cached.error {
                        // 缓存命中，无效结果
                        return Err(Status::internal(error.clone()));
                    }
                }
            }
        }
        
        // 限流检查
        self.check_rate_limit(key).await?;
        
        // 执行验证
        let validation_result = validation_fn();
        
        // 缓存结果
        {
            let mut cache = self.cache.write().await;
            let cached_result = match &validation_result {
                Ok(_) => CachedValidation {
                    is_valid: true,
                    error: None,
                    timestamp: Instant::now(),
                },
                Err(status) => CachedValidation {
                    is_valid: false,
                    error: Some(status.message().to_string()),
                    timestamp: Instant::now(),
                },
            };
            cache.insert(cache_key, cached_result);
        }
        
        // 返回验证结果
        validation_result
    }
    
    /// 限流检查
    async fn check_rate_limit(&self, key: &str) -> ValidationResult<()> {
        let rate_key = format!("rate:{}", key);
        let now = Instant::now();
        
        let is_limited = {
            let mut rate_limits = self.rate_limits.write().await;
            
            // 获取或创建计数器
            let timestamps = rate_limits.entry(rate_key.clone()).or_insert_with(Vec::new);
            
            // 移除过期的时间戳
            timestamps.retain(|ts| now.duration_since(*ts) < self.rate_limit_window);
            
            // 检查是否超出限制
            if timestamps.len() >= self.rate_limit_max_calls {
                true
            } else {
                // 添加当前时间戳
                timestamps.push(now);
                false
            }
        };
        
        if is_limited {
            warn!("Rate limit exceeded for key: {}", key);
            return Err(Status::resource_exhausted("请求过于频繁，请稍后再试"));
        }
        
        Ok(())
    }
    
    /// 清除缓存
    pub async fn invalidate_cache(&self, key: &str) {
        let cache_key = format!("validation:{}", key);
        let mut cache = self.cache.write().await;
        cache.remove(&cache_key);
    }
    
    /// 验证并记录事件
    pub async fn validate_and_log<F>(&self, operation: &str, subject_id: &str, object_id: Option<&str>, validation_fn: F) -> ValidationResult<()>
    where
        F: FnOnce() -> ValidationResult<()> + Send,
    {
        // 构建缓存键
        let key = match object_id {
            Some(obj_id) => format!("{}:{}:{}", operation, subject_id, obj_id),
            None => format!("{}:{}", operation, subject_id),
        };
        
        info!("开始验证 - 操作: {}, 用户: {}, 对象: {:?}", operation, subject_id, object_id);
        
        // 使用缓存验证
        let result = self.cache_validation(&key, validation_fn).await;
        
        match &result {
            Ok(_) => info!("验证通过 - 操作: {}, 用户: {}", operation, subject_id),
            Err(status) => warn!("验证失败 - 操作: {}, 用户: {}, 错误: {}", operation, subject_id, status.message()),
        }
        
        result
    }
} 