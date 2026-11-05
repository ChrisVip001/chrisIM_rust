#!/bin/bash
# 端到端压测的可重复执行入口
#
# 为什么需要这层包装：满载复测时最容易把「环境漂移」误判成「代码退化」。
# 常见污染源有三个，脚本按顺序清理，被测进程只看到干净状态：
#
# 1. 上一轮没退干净的压测进程仍在打流量；
# 2. 上一轮留在 Kafka 里的积压，会把 P99 拉到分钟级；
# 3. 令牌与 JWT 密钥不一致（服务端带着 JWT_SECRET 启动，
#    压测端没带），表现为全部 401，而不是性能问题。
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

# 1. 清理残留压测进程：两个压测同时跑时，CPU 争抢会让吞吐砍半
if pgrep -f e2e_bench >/dev/null 2>&1; then
  log "发现残留压测进程，先终止"
  pkill -f e2e_bench
  sleep 2
fi

# 2. 等积压排空：积压存在时测到的是「排空速度」，不是稳态延迟
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

# 3. 令牌密钥必须与服务端一致，否则整轮测量只会得到 401
if [ -z "${JWT_SECRET:-}" ]; then
  log "警告：未设置 JWT_SECRET。若服务端以 JWT_SECRET 启动，本轮将全部返回 401"
fi

started=$(date +%s)
JWT_SECRET=${JWT_SECRET:-} "$BENCH" --mode e2e \
  --rate "$RATE" --pairs "$PAIRS" --duration "$DURATION" --settle "$SETTLE"
status=$?
log "耗时 $(($(date +%s) - started))s，退出码 $status"

# 分段耗时只有在被测进程以 IM_STAGE_PROFILE=1 启动时才有内容
if [ -f "$STAGE_LOG" ] && grep -q '分段耗时' "$STAGE_LOG" 2>/dev/null; then
  log "最近 3 个窗口的服务端分段耗时："
  grep -o '"message":"分段耗时[^"]*"' "$STAGE_LOG" | tail -3
fi

exit "$status"
