//! WebSocket 网关压测工具
//!
//! 只压网关本身（不依赖 msg-server）：建立 N 条长连接，按限速持续发送上行消息，
//! 统计建连成功率、建连延迟分位、上行吞吐、异常断开数。
//!
//! 上行消息用 `Heartbeat` 类型，网关收到后直接短路返回，
//! 因此测到的是「接入层 + 连接管理」的容量，不掺杂下游 gRPC 的耗时。
//!
//! ## 用法
//!
//! ```text
//! # 1. 起依赖（Redis 用于 token 校验，Kafka/PG 不参与本压测）
//! docker compose up -d redis
//!
//! # 2. 编译并压测：1 万连接，每秒爬坡 500 条，持续 60 秒，每条连接每秒 1 条上行消息
//! cargo run --release -p msg-gateway --bin ws_bench -- \
//!     --config ./config/config.yaml \
//!     --connections 10000 \
//!     --ramp 500 \
//!     --duration 60 \
//!     --rate 1
//! ```
//!
//! 输出示例（数值随机器不同而变化，请以自己机器上的实测为准）：
//!
//! ```text
//! 建连成功 10000 / 10000，失败 0
//! 建连延迟  p50=1.8ms  p90=4.2ms  p99=11.6ms  max=48.3ms
//! 上行消息  598420 条，平均 9973 msg/s
//! 下行帧    12030 个（服务端心跳）
//! 异常断开  0
//! ```
//!
//! 压测会向 Redis 预置 `token:access:{用户}:{平台}` 键，
//! 请务必对着测试环境跑，不要指向线上库。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use runtime::auth::jwt::generate_token;
use runtime::config::AppConfig;
use contract::proto::message::{MsgType, PlatformType};

/// 命令行参数
struct Args {
    config: String,
    host: String,
    port: Option<u16>,
    connections: u64,
    ramp: u64,
    duration: u64,
    rate: u64,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Args {
            config: "./config/config.yaml".to_string(),
            host: "127.0.0.1".to_string(),
            port: None,
            connections: 1000,
            ramp: 200,
            duration: 30,
            rate: 1,
        };
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut index = 0;
        while index < raw.len() {
            let key = raw[index].clone();
            if !matches!(
                key.as_str(),
                "--config" | "--host" | "--port" | "--connections" | "--ramp" | "--duration" | "--rate"
            ) {
                return Err(format!("未知参数: {}", key));
            }

            index += 1;
            let value = raw
                .get(index)
                .cloned()
                .ok_or_else(|| format!("参数 {} 缺少取值", key))?;

            match key.as_str() {
                "--config" => args.config = value,
                "--host" => args.host = value,
                "--port" => {
                    args.port = Some(
                        value
                            .parse()
                            .map_err(|_| format!("--port 不是合法端口: {}", value))?,
                    )
                }
                "--connections" => {
                    args.connections = value
                        .parse()
                        .map_err(|_| format!("--connections 不是数字: {}", value))?
                }
                "--ramp" => {
                    args.ramp = value
                        .parse()
                        .map_err(|_| format!("--ramp 不是数字: {}", value))?
                }
                "--duration" => {
                    args.duration = value
                        .parse()
                        .map_err(|_| format!("--duration 不是数字: {}", value))?
                }
                "--rate" => {
                    args.rate = value
                        .parse()
                        .map_err(|_| format!("--rate 不是数字: {}", value))?
                }
                _ => unreachable!(),
            }

            index += 1;
        }
        Ok(args)
    }
}

/// 全局计数器
#[derive(Default)]
struct Counters {
    connect_ok: AtomicU64,
    connect_err: AtomicU64,
    msgs_sent: AtomicU64,
    frames_recv: AtomicU64,
    abnormal_close: AtomicU64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    let config = AppConfig::from_file(Some(&args.config))?;
    let port = args.port.unwrap_or(config.websocket.port);
    let platform = PlatformType::Web as i32;
    let jwt_config = config.gateway.auth.jwt.clone();

    println!(
        "开始压测: 连接数={}, 爬坡={}/s, 持续={}s, 每条连接 {} msg/s, 目标={}:{}",
        args.connections, args.ramp, args.duration, args.rate, args.host, port
    );

    // 预置 token，否则网关会以 4002 直接关闭连接
    let tokens = seed_tokens(&config, args.connections, platform, &jwt_config).await?;
    println!("已向 Redis 预置 {} 个 access token", tokens.len());

    let counters = Arc::new(Counters::default());
    let mut latencies: Vec<u64> = Vec::with_capacity(args.connections as usize);
    let mut handles = Vec::with_capacity(args.connections as usize);
    let stop = Arc::new(AtomicBool::new(false));

    // 每秒发起 ramp 条连接；ramp=0 表示不限制（间隔为 0）
    let ramp_interval =
        Duration::from_micros(1_000_000u64.checked_div(args.ramp).unwrap_or(0));
    // rate=0 表示只建连不发消息
    let per_conn_interval = 1_000_000u64
        .checked_div(args.rate)
        .map(Duration::from_micros);

    // 建连阶段
    for index in 0..args.connections {
        let (user_id, token) = match tokens.get(index as usize) {
            Some(item) => item.clone(),
            None => break,
        };
        let url = format!("ws://{}:{}/ws/{}", args.host, port, token);
        let counters = counters.clone();
        let writer_stop = stop.clone();
        let reader_stop = stop.clone();

        handles.push(tokio::spawn(async move {
            let started = Instant::now();
            let stream = match tokio_tungstenite::connect_async(&url).await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    counters.connect_err.fetch_add(1, Ordering::Relaxed);
                    eprintln!("连接 {} 失败: {}", user_id, e);
                    return None;
                }
            };

            let elapsed = started.elapsed().as_micros() as u64;
            counters.connect_ok.fetch_add(1, Ordering::Relaxed);

            let (mut sink, mut source) = stream.split();
            let (pong_tx, mut pong_rx) = tokio::sync::mpsc::channel::<Message>(8);

            let reader_counters = counters.clone();
            tokio::spawn(async move {
                while let Some(frame) = source.next().await {
                    match frame {
                        // Close 是正常关闭（含本工具主动 close），不计异常断开
                        Ok(Message::Close(_)) => break,
                        Err(_) => {
                            // 收尾阶段 1 万连接同时关闭产生的 RST 不是稳态异常断开
                            if !reader_stop.load(Ordering::Relaxed) {
                                reader_counters.abnormal_close.fetch_add(1, Ordering::Relaxed);
                            }
                            break;
                        }
                        Ok(Message::Ping(data)) => {
                            // 服务端心跳 Ping 必须回 Pong，否则连续 missed 次会被判死链
                            let _ = pong_tx.send(Message::Pong(data)).await;
                        }
                        Ok(_) => {
                            reader_counters.frames_recv.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });

            // 上行：用心跳类型，网关直接短路，不触发下游 gRPC。
            // 写端由单个任务独占，同时处理定时上行与 Pong 回复。
            let payload = serde_json::json!({
                "send_id": user_id,
                "receiver_id": user_id,
                "msg_type": MsgType::Heartbeat as i32,
                "content": [],
            })
            .to_string();
            let writer_counters = counters.clone();
            tokio::spawn(async move {
                let Some(interval) = per_conn_interval else {
                    return;
                };
                let mut ticker = tokio::time::interval(interval);
                loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            if writer_stop.load(Ordering::Relaxed) {
                                break;
                            }
                            if sink.send(Message::Text(payload.clone().into())).await.is_err() {
                                if !writer_stop.load(Ordering::Relaxed) {
                                    writer_counters.abnormal_close.fetch_add(1, Ordering::Relaxed);
                                }
                                break;
                            }
                            writer_counters.msgs_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Some(frame) = pong_rx.recv() => {
                            if sink.send(frame).await.is_err() {
                                if !writer_stop.load(Ordering::Relaxed) {
                                    writer_counters.abnormal_close.fetch_add(1, Ordering::Relaxed);
                                }
                                break;
                            }
                        }
                    }
                }
                let _ = sink.close().await;
            });

            Some(elapsed)
        }));

        if ramp_interval > Duration::ZERO {
            tokio::time::sleep(ramp_interval).await;
        }
    }

    // 收集建连延迟；读写任务已在各自连接内运行
    for handle in handles {
        if let Ok(Some(elapsed)) = handle.await {
            latencies.push(elapsed);
        }
    }
    latencies.sort_unstable();

    // 稳态压测：所有连接持续上行，直到停止标志置位
    let started = Instant::now();
    tokio::time::sleep(Duration::from_secs(args.duration)).await;
    stop.store(true, Ordering::Relaxed);
    // 留给各连接完成最后一次写入与主动关闭
    tokio::time::sleep(Duration::from_millis(300)).await;
    let elapsed = started.elapsed().as_secs_f64().max(0.001);

    let ok = counters.connect_ok.load(Ordering::Relaxed);
    let err = counters.connect_err.load(Ordering::Relaxed);
    let sent = counters.msgs_sent.load(Ordering::Relaxed);
    let frames = counters.frames_recv.load(Ordering::Relaxed);
    let abnormal = counters.abnormal_close.load(Ordering::Relaxed);

    println!();
    println!("建连成功 {} / {}，失败 {}", ok, args.connections, err);
    if !latencies.is_empty() {
        println!(
            "建连延迟  p50={}  p90={}  p99={}  max={}",
            format_micros(percentile(&latencies, 0.50)),
            format_micros(percentile(&latencies, 0.90)),
            format_micros(percentile(&latencies, 0.99)),
            format_micros(*latencies.last().unwrap()),
        );
    }
    println!("上行消息  {} 条，平均 {:.0} msg/s", sent, sent as f64 / elapsed);
    println!("下行帧    {} 个", frames);
    println!("异常断开  {}", abnormal);

    Ok(())
}

/// 生成并写入 token，返回 (用户ID, token) 列表
async fn seed_tokens(
    config: &AppConfig,
    count: u64,
    platform: i32,
    jwt_config: &runtime::configs::auth_config::JwtConfig,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let empty_extra: HashMap<String, String> = HashMap::new();
    let mut tokens = Vec::with_capacity(count as usize);

    for index in 0..count {
        let user_id = format!("bench-{}", index);
        let token = generate_token(
            &user_id,
            &user_id,
            "bench".to_string(),
            "bench",
            platform,
            empty_extra.clone(),
            jwt_config,
        )?;
        tokens.push((user_id, token));
    }

    let client = redis::Client::open(config.redis.url())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let ttl = jwt_config.expiry_seconds.max(600);

    for chunk in tokens.chunks(500) {
        let mut pipe = redis::pipe();
        for (user_id, token) in chunk {
            pipe.cmd("SET")
                .arg(format!("token:access:{}:{}", user_id, platform))
                .arg(token)
                .arg("EX")
                .arg(ttl)
                .ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
    }

    Ok(tokens)
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 * p) as usize).min(sorted.len() - 1);
    sorted[index]
}

fn format_micros(micros: u64) -> String {
    if micros >= 1000 {
        format!("{:.1}ms", micros as f64 / 1000.0)
    } else {
        format!("{}us", micros)
    }
}
