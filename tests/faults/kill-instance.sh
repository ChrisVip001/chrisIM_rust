#!/usr/bin/env bash
# 故障注入：随机 kill 一个业务实例，验证其余实例接管与恢复时间
#
# 用法：./tests/faults/kill-instance.sh <service> [健康检查命令]
#   service: relation-service | api-gateway | msg-server | msg-gateway
# 恢复判定：进程被拉起（由监督方/人工）或人工重启后健康检查通过
#
# 说明：kill -9 模拟最恶劣故障（无优雅停机）；同时给出优雅停机对照
set -uo pipefail

SVC="${1:?用法: kill-instance.sh <service> [health_cmd]}"
HEALTH="${2:-}"

pid=$(pgrep -f "target/release/$SVC" | head -1)
if [ -z "$pid" ]; then echo "FAIL  $SVC 未在运行"; exit 1; fi

t0=$(date +%s)
kill -9 "$pid"
echo "已 kill -9 $SVC (PID $pid)，开始计时..."

# 等待进程消失
for _ in $(seq 1 50); do
  pgrep -f "target/release/$SVC" >/dev/null || break
  sleep 0.1
done
echo "进程已消失: $(( $(date +%s) - t0 ))s"

if [ -n "$HEALTH" ]; then
  echo "等待恢复（执行: $HEALTH）..."
  for _ in $(seq 1 300); do
    if eval "$HEALTH" >/dev/null 2>&1; then
      echo "PASS  恢复耗时 $(( $(date +%s) - t0 ))s (RTO 预算 30s)"
      exit 0
    fi
    sleep 0.1
  done
  echo "FAIL  300s 内未恢复"
  exit 1
fi
echo "进程已终止，请重启后人工确认接管行为（RTO 目标 < 30s）"
