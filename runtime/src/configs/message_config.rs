use serde::Deserialize;

/// 消息存储配置（统一读扩散）
///
/// 对应 `docs/sql/init.sql`。
///
/// 设计前提：一条消息恒定写 1 行元数据，写入成本与群规模无关。
/// 因此这里不再有「写扩散阈值」之类的分流参数 —— 分流本身是写放大的来源，
/// 已被整体移除。
#[derive(Debug, Deserialize, Clone)]
pub struct MessageStorageConfig {
    /// 正文内联上限（字节）
    ///
    /// 不超过该值的正文直接写进 `msg_meta.inline_content`，拉取时无需回表；
    /// 超过的写入 `msg_content`，由客户端按需批量补取。
    #[serde(default = "default_inline_content_max_bytes")]
    pub inline_content_max_bytes: usize,

    /// 默认拉取条数
    #[serde(default = "default_fetch_limit")]
    pub fetch_default_limit: i64,

    /// 单次拉取硬上限，防止客户端请求超大页把连接池占满
    #[serde(default = "default_fetch_max_limit")]
    pub fetch_max_limit: i64,

    /// Redis 缓存的最近消息条数
    ///
    /// 用于规避「刚发就拉不到」的复制延迟，并承接绝大多数历史拉取。
    #[serde(default = "default_hot_cache_messages")]
    pub hot_cache_messages: i64,

    /// 历史归档阈值（天），超过后由归档任务迁移到归档存储
    #[serde(default = "default_archive_after_days")]
    pub archive_after_days: i64,

    /// 是否使用单语句多行批量写入
    ///
    /// 开启后禁止在应用层逐行 insert，批量写必须一次往返完成。
    #[serde(default = "default_true")]
    pub batch_insert: bool,

    /// 历史拉取是否走只读副本
    ///
    /// 初期可关闭；开启后需配合 Redis 最近消息缓存，否则会读到复制延迟内的空洞。
    #[serde(default)]
    pub read_from_replica: bool,
}

impl Default for MessageStorageConfig {
    fn default() -> Self {
        Self {
            inline_content_max_bytes: default_inline_content_max_bytes(),
            fetch_default_limit: default_fetch_limit(),
            fetch_max_limit: default_fetch_max_limit(),
            hot_cache_messages: default_hot_cache_messages(),
            archive_after_days: default_archive_after_days(),
            batch_insert: default_true(),
            read_from_replica: false,
        }
    }
}

impl MessageStorageConfig {
    /// 正文是否内联存储
    pub fn should_inline(&self, content_len: usize) -> bool {
        content_len <= self.inline_content_max_bytes
    }

    /// 把客户端请求的条数收敛到 [1, fetch_max_limit]
    pub fn clamp_limit(&self, requested: i64) -> i64 {
        let limit = if requested <= 0 {
            self.fetch_default_limit
        } else {
            requested
        };
        limit.clamp(1, self.fetch_max_limit.max(1))
    }

    /// 是否已达到归档条件
    pub fn should_archive(&self, age_days: i64) -> bool {
        self.archive_after_days > 0 && age_days >= self.archive_after_days
    }
}

fn default_inline_content_max_bytes() -> usize {
    1024
}

fn default_fetch_limit() -> i64 {
    50
}

fn default_fetch_max_limit() -> i64 {
    500
}

fn default_hot_cache_messages() -> i64 {
    200
}

fn default_archive_after_days() -> i64 {
    365
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_read_spread_oriented() {
        let cfg = MessageStorageConfig::default();
        assert_eq!(cfg.inline_content_max_bytes, 1024);
        assert_eq!(cfg.fetch_default_limit, 50);
        assert_eq!(cfg.fetch_max_limit, 500);
        assert!(!cfg.read_from_replica);
    }

    #[test]
    fn inline_decision_includes_boundary() {
        let cfg = MessageStorageConfig::default();
        assert!(cfg.should_inline(0));
        assert!(cfg.should_inline(1024));
        assert!(!cfg.should_inline(1025));
    }

    #[test]
    fn clamp_limit_falls_back_to_default() {
        let cfg = MessageStorageConfig::default();
        assert_eq!(cfg.clamp_limit(0), 50);
        assert_eq!(cfg.clamp_limit(-1), 50);
        assert_eq!(cfg.clamp_limit(20), 20);
        assert_eq!(cfg.clamp_limit(10_000), 500);
    }

    #[test]
    fn archive_threshold_disabled_when_zero() {
        let mut cfg = MessageStorageConfig::default();
        assert!(cfg.should_archive(365));
        assert!(!cfg.should_archive(364));
        cfg.archive_after_days = 0;
        assert!(!cfg.should_archive(100_000));
    }
}
