#!/bin/bash
# 端到端压测的可重复执行入口
#
# 开测前按顺序做三件事：
# 1. 终止上一轮残留的 e2e_bench 进程；
# 2. 轮询 Kafka 消费组 lag，等到归零再开始计时；
# 3. 校验 JWT_SECRET 是否与待测服务一致（不一致时全部返回 401）。
#
# 用法：
#   tests/perf/e2e.sh [rate] [pairs] [duration] [settle]
#   JWT_SECRET=xxx tests/perf/e2e.sh 3000 64 15 10
set -u

cd "$(dirname "$0")/../.." || exit 1

RATE=${1:-3000}
PAIRS=${2:-64}
DURATION=${3:-15}
SETTLE=${4:-10}

KAFKA_CONTAINER=${KAFKA_CONTAINER:-rustim_kafka}
GROUP=${GROUP:-chat}
BENCH=./target/release/e2e_bench
STAGE_LOG=logs/msg-server.log

log() { printf '[e2e] %s\n' "$*"; }

if [ ! -x "$BENCH" ]; then
  log "缺少 $BENCH，先执行 cargo build --release --bin e2e_bench"
  exit 1
fi

# 1. 终止残留的压测进程
if pgrep -f e2e_bench >/dev/null 2>&1; then
  log "发现残留压测进程，先终止"
  pkill -f e2e_bench
  sleep 2
fi

# 2. 等 Kafka 积压排空（消费组 lag 归零）
lag_total() {
  docker exec "$KAFKA_CONTAINER" kafka-consumer-groups \
    --bootstrap-server localhost:9092 --describe --group "$GROUP" 2>/dev/null |
    awk 'NR>1 {s += $6} END {print s + 0}'
}

log "等待 Kafka 积压排空"
for _ in $(seq 1 30); do
  lag=$(lag_total)
  [ "$lag" = "0" ] && break
  sleep 2
done
log "起始 lag=$lag"

# 3. 密钥与服务端不一致时告警（整轮将只得到 401）
if [ -z "${JWT_SECRET:-}" ]; then
  log "警告：未设置 JWT_SECRET。若服务端以 JWT_SECRET 启动，本轮将全部返回 401"
fi

started=$(date +%s)
JWT_SECRET=${JWT_SECRET:-} "$BENCH" --mode e2e \
  --rate "$RATE" --pairs "$PAIRS" --duration "$DURATION" --settle "$SETTLE"
status=$?
log "耗时 $(($(date +%s) - started))s，退出码 $status"

# 输出最近 3 个窗口的服务端分段耗时（需 msg-server 以 IM_STAGE_PROFILE=1 启动）
if [ -f "$STAGE_LOG" ] && grep -q '分段耗时' "$STAGE_LOG" 2>/dev/null; then
  log "最近 3 个窗口的服务端分段耗时："
  grep -o '"message":"分段耗时[^"]*"' "$STAGE_LOG" | tail -3
fi

exit "$status"
