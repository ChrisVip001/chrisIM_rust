//! 端到端性能压测（黑盒）
//!
//! 与 `ws_bench` 互补：`ws_bench` 上行发 Heartbeat，被网关短路，只测接入层容量；
//! 本工具走完整业务链路，用于验收三个此前未测的指标：
//!
//! - `e2e`：HTTP `POST /api/chat/send` → msg-server → Kafka → 落库 → 推送 → 接收端 WS，
//!   按推送帧里的 `create_time`（api-gateway 打点）计算端到端延迟分位数；
//! - `qps`：按目标 QPS 恒定速率打同一写路径，同时断言吞吐与错误率
//!   （无节制打满只会测出限流上限，不是系统容量）。
//!
//! 前置：四个服务 + PG/Redis/Kafka/Consul 已运行；压测用户 token 与好友关系会写入 Redis，
//! 因此只能对测试环境运行。JWT_SECRET 必须与服务端一致。
//!
//! ## 用法
//! ```text
//! cargo run --release -p msg-gateway --bin e2e_bench -- --mode e2e --rate 500 --pairs 16 --duration 30
//! cargo run --release -p msg-gateway --bin e2e_bench -- --mode qps --duration 30
//! ```
//!
//! `--pairs` 控制会话对数：消费端按会话分组、组内串行保序，因此单会话只能测出
//! 「一个会话的上限」，多会话并发才代表真实容量。

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};
use tokio_tungstenite::tungstenite::Message;

use contract::proto::message::{MsgType, PlatformType};
use runtime::auth::jwt::generate_token;
use runtime::config::AppConfig;

const SENDER: &str = "bench-e2e-sender";
const RECEIVER: &str = "bench-e2e-receiver";

struct Args {
    config: String,
    host: String,
    api_port: u16,
    ws_port: u16,
    mode: String,
    rate: Option<u64>,
    duration: u64,
    concurrency: usize,
    target_qps: u64,
    settle: u64,
    /// 会话对数：单会话在消费端强制串行保序，只代表「单个会话上限」，
    /// 多会话并发才是真实 IM 负载
    pairs: usize,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut a = Args {
            config: "./config/config.yaml".to_string(),
            host: "127.0.0.1".to_string(),
            api_port: 8000,
            ws_port: 8085,
            mode: "e2e".to_string(),
            rate: None,
            duration: 30,
            concurrency: 128,
            target_qps: 3000,
            settle: 5,
            pairs: 16,
        };
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < raw.len() {
            let key = raw[i].clone();
            i += 1;
            let v = raw.get(i).cloned().ok_or_else(|| format!("{} 缺少取值", key))?;
            i += 1;
            match key.as_str() {
                "--config" => a.config = v,
                "--host" => a.host = v,
                "--api-port" => a.api_port = v.parse().map_err(|_| "非法 --api-port")?,
                "--ws-port" => a.ws_port = v.parse().map_err(|_| "非法 --ws-port")?,
                "--mode" => a.mode = v,
                "--rate" => a.rate = Some(v.parse().map_err(|_| "非法 --rate")?),
                "--duration" => a.duration = v.parse().map_err(|_| "非法 --duration")?,
                "--concurrency" => a.concurrency = v.parse().map_err(|_| "非法 --concurrency")?,
                "--target-qps" => a.target_qps = v.parse().map_err(|_| "非法 --target-qps")?,
                "--settle" => a.settle = v.parse().map_err(|_| "非法 --settle")?,
                "--pairs" => a.pairs = v.parse().map_err(|_| "非法 --pairs")?,
                _ => return Err(format!("未知参数: {}", key)),
            }
        }
        if !matches!(a.mode.as_str(), "e2e" | "qps") {
            return Err("--mode 只能是 e2e 或 qps".to_string());
        }
        if a.effective_rate() == 0 || a.duration == 0 || a.concurrency == 0 || a.pairs == 0 {
            return Err("rate/duration/concurrency 必须大于 0".to_string());
        }
        Ok(a)
    }

    /// e2e 默认低速率看延迟，qps 默认按目标 QPS 恒定注入
    fn effective_rate(&self) -> u64 {
        self.rate
            .unwrap_or(if self.mode == "qps" { self.target_qps } else { 200 })
    }
}

#[derive(Default)]
struct SendStats {
    ok: AtomicU64,
    limited: AtomicU64,
    server_error: AtomicU64,
    other_error: AtomicU64,
}

#[derive(Default)]
struct RecvStats {
    latencies: Vec<u64>,
    seen: HashSet<String>,
    duplicates: u64,
    frames: u64,
}

impl RecvStats {
    /// 预分配到预期帧数：满载时每帧都触发 Vec/HashSet 扩容，
    /// 扩容期间持有全局锁，会把测量到的尾延迟算到被测系统头上。
    fn with_capacity(frames: usize) -> Self {
        Self {
            latencies: Vec::with_capacity(frames),
            seen: HashSet::with_capacity(frames),
            duplicates: 0,
            frames: 0,
        }
    }
}

/// 预置 N 对会话（发送方 + 接收方）：JWT + Redis access token + 好友关系。
/// 好友关系直接写缓存，权限校验走「缓存命中」路径（正常业务路径），不产生额外 RPC。
async fn setup(
    config: &AppConfig,
    pairs: usize,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let platform = PlatformType::Web as i32;
    let jwt = &config.gateway.auth.jwt;
    let extra = std::collections::HashMap::<String, String>::new();
    let ttl = jwt.expiry_seconds.max(600);

    let mut result = Vec::with_capacity(pairs);
    let mut pipe = redis::pipe();
    for index in 0..pairs {
        let sender = format!("{}-{}", SENDER, index);
        let receiver = format!("{}-{}", RECEIVER, index);
        let sender_token = generate_token(
            &sender,
            &sender,
            "bench".to_string(),
            "bench",
            platform,
            extra.clone(),
            jwt,
        )?;
        let receiver_token = generate_token(
            &receiver,
            &receiver,
            "bench".to_string(),
            "bench",
            platform,
            extra.clone(),
            jwt,
        )?;

        for (user, token) in [(&sender, &sender_token), (&receiver, &receiver_token)] {
            pipe.cmd("SET")
                .arg(format!("token:access:{}:{}", user, platform))
                .arg(token)
                .arg("EX")
                .arg(ttl)
                .ignore();
        }
        pipe.cmd("SADD")
            .arg(format!("user_friends:{}", sender))
            .arg(&receiver)
            .ignore();
        pipe.cmd("SADD")
            .arg(format!("user_friends:{}", receiver))
            .arg(&sender)
            .ignore();
        result.push((sender_token, receiver_token));
    }

    let client = redis::Client::open(config.redis.url())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    pipe.query_async::<()>(&mut conn).await?;
    Ok(result)
}

/// 发一条消息；返回 HTTP 状态码与「是否业务成功」
async fn send_once(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    receiver: &str,
    local_id: &str,
) -> (u16, bool) {
    let body = json!({
        "receiverId": receiver,
        "content": "phase2-bench",
        "localId": local_id,
        "contentType": "Text",
        "msgType": MsgType::SingleMsg as i32,
    });
    match client
        .post(url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ok = status == 200
                && resp
                    .json::<Value>()
                    .await
                    .ok()
                    .and_then(|v| v["success"].as_bool())
                    .unwrap_or(false);
            (status, ok)
        }
        Err(_) => (0, false),
    }
}

fn record(stats: &Arc<SendStats>, status: u16, ok: bool) {
    if ok {
        stats.ok.fetch_add(1, Ordering::Relaxed);
    } else {
        match status {
            429 => stats.limited.fetch_add(1, Ordering::Relaxed),
            500..=599 => stats.server_error.fetch_add(1, Ordering::Relaxed),
            _ => stats.other_error.fetch_add(1, Ordering::Relaxed),
        };
    }
}

/// 接收端 WS：解析推送帧，按 server_id 去重后记录端到端延迟
async fn receiver_task(url: String, recv: Arc<Mutex<RecvStats>>, min_create_time: i64) {
    let stream = match tokio_tungstenite::connect_async(&url).await {
        Ok((s, _)) => s,
        Err(e) => {
            eprintln!("接收端连接失败: {}", e);
            return;
        }
    };
    let (mut sink, mut source) = stream.split();
    let (pong_tx, mut pong_rx) = tokio::sync::mpsc::channel::<Message>(8);

    // 写任务只负责回 Pong：服务端 30s 心跳 × missed 2 会把不回 Pong 的连接判死链
    tokio::spawn(async move {
        while let Some(frame) = pong_rx.recv().await {
            if sink.send(frame).await.is_err() {
                break;
            }
        }
    });

    while let Some(frame) = source.next().await {
        match frame {
            Ok(Message::Ping(data)) => {
                let _ = pong_tx.send(Message::Pong(data)).await;
            }
            Ok(Message::Text(text)) => {
                let value: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let msg = &value["conversations"][0]["recent_messages"][0];
                let server_id = msg["server_id"].as_str().unwrap_or("").to_string();
                let create_time = msg["create_time"].as_i64().unwrap_or(0);
                // 忽略上一轮压测残留的迟到推送，只统计本次注入的消息
                if server_id.is_empty() || create_time < min_create_time {
                    continue;
                }
                let now = now_millis();
                let mut stats = recv.lock().await;
                stats.frames += 1;
                if !stats.seen.insert(server_id) {
                    stats.duplicates += 1;
                    continue;
                }
                stats.latencies.push((now - create_time).max(0) as u64);
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }
}

async fn run_e2e(a: &Args, client: reqwest::Client, pairs: Vec<(String, String)>) -> i32 {
    let url = format!("http://{}:{}/api/chat/send", a.host, a.api_port);
    let expected_frames = (a.effective_rate() * a.duration) as usize + 1024;
    let recv = Arc::new(Mutex::new(RecvStats::with_capacity(expected_frames)));
    let send = Arc::new(SendStats::default());
    let min_create_time = now_millis() - 1000;

    let mut readers = Vec::with_capacity(pairs.len());
    for (_, receiver_token) in &pairs {
        let ws_url = format!("ws://{}:{}/ws/{}", a.host, a.ws_port, receiver_token);
        let recv_for_task = recv.clone();
        readers.push(tokio::spawn(async move {
            receiver_task(ws_url, recv_for_task, min_create_time).await;
        }));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    let seq = Arc::new(AtomicU64::new(0));
    let sem = Arc::new(Semaphore::new(a.concurrency));
    let deadline = Instant::now() + Duration::from_secs(a.duration);
    let mut rate = RateLimiter::new(a.effective_rate());
    let started = Instant::now();

    while Instant::now() < deadline {
        for _ in 0..rate.next_batch().await {
            let permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let n = seq.fetch_add(1, Ordering::Relaxed);
            let idx = n as usize % pairs.len();
            let client = client.clone();
            let url = url.clone();
            let token = pairs[idx].0.clone();
            let receiver = format!("{}-{}", RECEIVER, idx);
            let send = send.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let (status, ok) =
                    send_once(&client, &url, &token, &receiver, &format!("e2e-{}", n)).await;
                record(&send, status, ok);
            });
        }
    }
    // 计时窗口只覆盖注入期，不包含等在途请求与 settle
    let elapsed = started.elapsed().as_secs_f64();

    // 等在途请求结束，再留一段余量给 Kafka 消费与推送
    let _ = sem.acquire_many(a.concurrency as u32).await;
    tokio::time::sleep(Duration::from_secs(a.settle)).await;
    for reader in readers {
        reader.abort();
    }
    let ok = send.ok.load(Ordering::Relaxed);
    let stats = recv.lock().await;
    let mut lat = stats.latencies.clone();
    lat.sort_unstable();

    println!(
        "模式      e2e（HTTP 写路径 → 接收端 WS），会话对 {}",
        pairs.len()
    );
    println!(
        "发送      {} 成功 / {:.0}s，{:.0} msg/s；429={} 5xx={} 其他失败={}",
        ok,
        elapsed,
        ok as f64 / elapsed,
        send.limited.load(Ordering::Relaxed),
        send.server_error.load(Ordering::Relaxed),
        send.other_error.load(Ordering::Relaxed),
    );
    println!(
        "推送帧    {} 个，去重后 {} 条，重复 {} 条，到达率 {:.2}%",
        stats.frames,
        lat.len(),
        stats.duplicates,
        if ok == 0 { 0.0 } else { lat.len() as f64 * 100.0 / ok as f64 },
    );
    if lat.is_empty() {
        println!("FAIL  没有收到任何推送帧，检查权限校验 / 推送路由 / Kafka 消费");
        return 1;
    }
    println!(
        "端到端    p50={}ms p90={}ms p99={}ms max={}ms（样本 {}）",
        percentile(&lat, 0.50),
        percentile(&lat, 0.90),
        percentile(&lat, 0.99),
        lat[lat.len() - 1],
        lat.len(),
    );
    let p99 = percentile(&lat, 0.99);
    if p99 < 200 {
        println!("PASS  端到端 P99 {}ms < 200ms", p99);
        0
    } else {
        println!("FAIL  端到端 P99 {}ms ≥ 200ms", p99);
        1
    }
}

async fn run_qps(a: &Args, client: reqwest::Client, token: String) -> i32 {
    let url = format!("http://{}:{}/api/chat/send", a.host, a.api_port);
    let send = Arc::new(SendStats::default());
    let latencies = Arc::new(Mutex::new(Vec::<u64>::new()));
    let seq = Arc::new(AtomicU64::new(0));
    let sem = Arc::new(Semaphore::new(a.concurrency));
    let rate = a.effective_rate();

    // 恒定速率注入：无节制打满只会测出限流上限，不是系统容量。
    // 达标条件是「目标 QPS 下几乎不出现 429/5xx」，而非「有多少请求被放行」。
    let deadline = Instant::now() + Duration::from_secs(a.duration);
    let mut limiter = RateLimiter::new(rate);
    let started = Instant::now();

    while Instant::now() < deadline {
        for _ in 0..limiter.next_batch().await {
            let permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let n = seq.fetch_add(1, Ordering::Relaxed);
            let client = client.clone();
            let url = url.clone();
            let token = token.clone();
            let send = send.clone();
            let latencies = latencies.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let t0 = Instant::now();
                let (status, ok) = send_once(
                    &client,
                    &url,
                    &token,
                    &format!("{}-0", RECEIVER),
                    &format!("qps-{}", n),
                )
                .await;
                record(&send, status, ok);
                if ok {
                    latencies.lock().await.push(t0.elapsed().as_millis() as u64);
                }
            });
        }
    }
    // 计时窗口只覆盖注入期，不包含等在途请求的时间
    let elapsed = started.elapsed().as_secs_f64();
    let _ = sem.acquire_many(a.concurrency as u32).await;

    let ok = send.ok.load(Ordering::Relaxed);
    let limited = send.limited.load(Ordering::Relaxed);
    let server_error = send.server_error.load(Ordering::Relaxed);
    let other = send.other_error.load(Ordering::Relaxed);
    let total = ok + limited + server_error + other;
    let error_rate = if total == 0 { 100.0 } else { (total - ok) as f64 * 100.0 / total as f64 };
    let qps = (ok as f64 / elapsed) as u64;
    let mut lat = latencies.lock().await.clone();
    lat.sort_unstable();

    println!("模式      qps（HTTP 写路径恒定负载）");
    println!(
        "吞吐      注入 {} QPS，{} 成功 / {:.2}s = {:.0} QPS（目标 {}）",
        rate, ok, elapsed, qps, a.target_qps,
    );
    println!(
        "状态码    200={} 429={} 5xx={} 其他失败={}，错误率 {:.3}%",
        ok, limited, server_error, other, error_rate,
    );
    if !lat.is_empty() {
        println!(
            "HTTP 往返 p50={}ms p99={}ms max={}ms",
            percentile(&lat, 0.50),
            percentile(&lat, 0.99),
            lat[lat.len() - 1],
        );
    }
    if qps >= a.target_qps && error_rate < 0.1 {
        println!("PASS  {} QPS ≥ {} 且错误率 {:.3}% < 0.1%", qps, a.target_qps, error_rate);
        0
    } else {
        println!("FAIL  {} QPS / 错误率 {:.3}% 未达标", qps, error_rate);
        1
    }
}

/// 恒定速率注入器：每 10ms 一个周期，用小数余数保证任意速率都精确。
/// 单条消息一个 timer tick 在 3000/s 时会被调度抖动拉低约 1%。
struct RateLimiter {
    ticker: tokio::time::Interval,
    debt: f64,
    per_tick: f64,
}

impl RateLimiter {
    fn new(rate: u64) -> Self {
        let mut ticker = tokio::time::interval(Duration::from_millis(10));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self {
            ticker,
            debt: 0.0,
            per_tick: rate as f64 / 100.0,
        }
    }

    async fn next_batch(&mut self) -> usize {
        self.ticker.tick().await;
        self.debt += self.per_tick;
        let count = self.debt.floor();
        self.debt -= count;
        count as usize
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 * p) as usize).min(sorted.len() - 1);
    sorted[index]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = Args::parse()?;
    let config = AppConfig::from_file(Some(&a.config))?;
    a.api_port = config.server.port;
    a.ws_port = config.websocket.port;

    println!("目标 {}:{} (HTTP) / {}:{} (WS)，模式={}", a.host, a.api_port, a.host, a.ws_port, a.mode);
    let pairs = setup(&config, a.pairs).await?;
    println!("已预置 {} 对会话的 token 与好友关系", pairs.len());

    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(a.concurrency)
        .timeout(Duration::from_secs(10))
        .build()?;

    let code = match a.mode.as_str() {
        "e2e" => run_e2e(&a, client, pairs).await,
        "qps" => run_qps(&a, client, pairs[0].0.clone()).await,
        _ => unreachable!(),
    };
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
