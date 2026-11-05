#!/usr/bin/env bash
# 稳态 RSS 采样：在 1 万连接挂载期间观测业务进程内存
#
# 用法：./tests/perf/rss.sh <进程名> [采样秒数] [预算KB]
#   ./tests/perf/rss.sh msg-gateway 60 524288
# 前置：目标服务已运行，且 ws_bench 正在保持连接（另开终端运行）。
# macOS 的 `ps -o rss=` 单位为 KB；512MB = 524288KB。
set -uo pipefail

PROC="${1:?用法: rss.sh <进程名> [采样秒数] [预算KB]}"
DURATION="${2:-60}"
BUDGET_KB="${3:-524288}"

pid=$(pgrep -f "target/release/${PROC}" | head -1)
if [ -z "$pid" ]; then
  echo "FAIL  未找到 ${PROC} 进程"
  exit 1
fi

echo "采样 ${PROC} (PID ${pid})，${DURATION}s，预算 ${BUDGET_KB}KB"
max=0
sum=0
count=0
for _ in $(seq 1 "$DURATION"); do
  rss=$(ps -o rss= -p "$pid" | tr -d ' ')
  if [ -n "$rss" ]; then
    max=$(( rss > max ? rss : max ))
    sum=$(( sum + rss ))
    count=$(( count + 1 ))
  fi
  sleep 1
done

if [ "$count" -eq 0 ]; then
  echo "FAIL  进程在采样期间消失"
  exit 1
fi

avg=$(( sum / count ))
echo "RSS  avg=$((avg / 1024))MB  max=$((max / 1024))MB  样本 ${count} 个"
if [ "$max" -le "$BUDGET_KB" ]; then
  echo "PASS  峰值 $((max / 1024))MB < $((BUDGET_KB / 1024))MB"
  exit 0
fi
echo "FAIL  峰值 $((max / 1024))MB ≥ $((BUDGET_KB / 1024))MB"
exit 1
