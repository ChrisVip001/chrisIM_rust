//! 热路径分段耗时采样
//!
//! 端到端 P99 不达标时，必须先回答「延迟累积在哪一段」，否则只能靠调参猜。
//! 消费链路跨两个 crate（`msg-server` 编排、`msg-storage` 落库），
//! 因此采样器放在两者共同依赖的运行时层，按固定阶段名记录。
//!
//! ```text
//! create_time ─Wait─▶ 进入处理 ─Idem─▶ 幂等查询 ─Seq─▶ 号段 ─Insert─▶ 落库
//!                              └───────────── Push ─▶ 推送完成
//! ```
//!
//! ## 代价与开关
//!
//! `IM_STAGE_PROFILE=1` 显式开启，默认关闭。关闭时 `record` 只读一个
//! `OnceLock` 就返回，不分配、不加锁，可以直接留在生产热路径上
//! （排查性能问题时不必重新编译部署）。
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tracing::info;

/// 采样率分母：每个阶段每 N 次记录取一次
const SAMPLE_EVERY: u64 = 16;

/// 采样窗口长度：定期输出并清空，保证分位数反映当前窗口而不是累计值
const REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// 可观测的阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 消息从生产端 `create_time` 到进入消费处理的排队时间（Kafka 积压 + 调度）
    Wait,
    /// 幂等预查询
    Idem,
    /// 会话号段领取（含号段跨界的 PG 回写）
    Seq,
    /// 单条消息写入（元数据 + 成员 + 会话水位）
    Insert,
    /// 推送（路由查询 + 网关 gRPC）
    Push,
    /// 端到端总耗时
    Total,
    /// 生产段：消息从 `create_time` 到被 broker 接收（api-gateway → Kafka）
    ///
    /// 单看 [`Stage::Wait`] 无法区分「消息还没进 Kafka」和「进了 Kafka 没被消费」，
    /// 这两者的修复方向完全相反，所以单独打一段。
    Produce,
}

impl Stage {
    const ALL: [Stage; 7] = [
        Stage::Wait,
        Stage::Idem,
        Stage::Seq,
        Stage::Insert,
        Stage::Push,
        Stage::Total,
        Stage::Produce,
    ];

    fn index(self) -> usize {
        match self {
            Stage::Wait => 0,
            Stage::Idem => 1,
            Stage::Seq => 2,
            Stage::Insert => 3,
            Stage::Push => 4,
            Stage::Total => 5,
            Stage::Produce => 6,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Stage::Wait => "等待",
            Stage::Idem => "幂等",
            Stage::Seq => "号段",
            Stage::Insert => "写入",
            Stage::Push => "推送",
            Stage::Total => "端到端",
            Stage::Produce => "生产",
        }
    }
}

type Buckets = [Vec<u64>; 7];

static ENABLED: OnceLock<bool> = OnceLock::new();
static BUCKETS: OnceLock<Mutex<Buckets>> = OnceLock::new();
static SEEN: OnceLock<AtomicU64> = OnceLock::new();
static REPORTING: OnceLock<()> = OnceLock::new();

/// 采样是否开启
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var("IM_STAGE_PROFILE").as_deref() == Ok("1"))
}

/// 记录一次阶段耗时
///
/// 每个阶段独立采样，因此各阶段样本不来自同一批消息；
/// 分位数仍然有效，只是不能用来做「单条消息的分段占比」对账。
pub fn record(stage: Stage, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let seen = SEEN
        .get_or_init(|| AtomicU64::new(0))
        .fetch_add(1, Ordering::Relaxed);
    if seen % SAMPLE_EVERY != 0 {
        return;
    }
    let Some(buckets) = BUCKETS.get() else {
        return;
    };
    let Ok(mut buckets) = buckets.lock() else {
        return;
    };
    buckets[stage.index()].push(elapsed.as_micros() as u64);
}

/// 供调用方复用的计时器：`start` 后 `lap` 一次记录一段
///
/// 关闭采样时所有方法都是空操作，不会因为诊断代码引入分支开销。
pub struct Lap {
    enabled: bool,
    started: Instant,
}

impl Lap {
    pub fn start() -> Self {
        Self {
            enabled: enabled(),
            started: Instant::now(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 记录从上一个锚点（或 `start`）到现在的耗时，并重置锚点
    pub fn lap(&mut self, stage: Stage) -> Duration {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.started);
        self.started = now;
        if self.enabled {
            record(stage, elapsed);
        }
        elapsed
    }

    /// 丢弃当前锚点之前的时间，不记录
    pub fn reset(&mut self) {
        self.started = Instant::now();
    }

    /// 记录一个绝对时刻距当前的时间，不重置锚点
    pub fn since(&self, stage: Stage, anchor: Instant) {
        if self.enabled {
            record(stage, anchor.elapsed());
        }
    }
}

/// 启动后台聚合任务（每个进程只启动一次）
///
/// 反复调用安全：同一进程内的多个消费者共享采样表，
/// 只允许第一个调用者拉起输出任务。
pub fn spawn_reporter() {
    if !enabled() {
        return;
    }
    if REPORTING.set(()).is_err() {
        return;
    }
    BUCKETS.get_or_init(|| Mutex::new(Default::default()));

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REPORT_INTERVAL).await;
            let Some(buckets) = BUCKETS.get() else {
                return;
            };
            let Ok(mut buckets) = buckets.lock() else {
                return;
            };
            if buckets.iter().all(|b| b.is_empty()) {
                continue;
            }

            let parts: Vec<String> = Stage::ALL
                .iter()
                .map(|&stage| {
                    let values = &mut buckets[stage.index()];
                    format!(
                        "{}={:.1}/{:.1}/{:.1}",
                        stage.label(),
                        percentile(values, 0.50) as f64 / 1000.0,
                        percentile(values, 0.90) as f64 / 1000.0,
                        percentile(values, 0.99) as f64 / 1000.0,
                    )
                })
                .collect();

            info!("分段耗时(p50/p90/p99 ms) {} 样本={}", parts.join(" "), buckets[Stage::Wait.index()].len());

            for bucket in buckets.iter_mut() {
                bucket.clear();
            }
        }
    });
}

fn percentile(values: &mut [u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let idx = ((values.len() - 1) as f64 * p).round() as usize;
    values[idx.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_uses_nearest_rank_and_tolerates_empty() {
        let mut values = vec![1, 2, 3, 4, 5];
        assert_eq!(percentile(&mut values, 0.50), 3);
        assert_eq!(percentile(&mut values, 0.99), 5);
        assert_eq!(
            percentile(&mut Vec::new(), 0.50),
            0,
            "空窗口不能 panic，也不能返回非零"
        );
    }

    #[test]
    fn laps_accumulate_without_losing_time() {
        let mut lap = Lap::start();
        std::thread::sleep(Duration::from_millis(2));
        let first = lap.lap(Stage::Idem);
        std::thread::sleep(Duration::from_millis(2));
        let second = lap.lap(Stage::Seq);
        assert!(first >= Duration::from_millis(2));
        assert!(second >= Duration::from_millis(2));
    }

    #[test]
    fn recording_is_a_noop_when_disabled() {
        if enabled() {
            return;
        }
        // 未开启时不应创建采样表，避免常驻内存
        record(Stage::Total, Duration::from_millis(1));
        assert!(BUCKETS.get().is_none(), "关闭采样时不应分配采样表");
    }
}
