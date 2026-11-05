#!/usr/bin/env bash
# 故障注入：测量实例被 Consul 从健康列表摘除的时间（RTO 的可自动验证部分）
#
# 用法：./tests/faults/rto-consul.sh <consul服务名> <进程名> [kill9|term] [预算秒数]
#   示例：./tests/faults/rto-consul.sh ws msg-gateway kill9 20
#         ./tests/faults/rto-consul.sh ws msg-gateway term  20
#   kill9：模拟最恶劣故障，验证 TTL 过期 + 发现刷新的摘流时间（预算 20s）
#   term ：优雅停机，验证 deregister 立即摘流且 TTL 上报任务不会把实例"复活"
#
# 前置：目标服务已运行并注册为 passing；Consul 可从本机访问；需要 jq / curl
# 说明：发现端刷新间隔 5s 由配置与单元测试保证，不由本脚本测量。
set -uo pipefail

SVC="${1:?用法: rto-consul.sh <consul服务名> <进程名> [kill9|term] [预算秒数]}"
PROC="${2:?用法: rto-consul.sh <consul服务名> <进程名> [kill9|term] [预算秒数]}"
MODE="${3:-kill9}"
BUDGET="${4:-20}"
CONSUL="${CONSUL_ADDR:-http://127.0.0.1:8500}"
URL="$CONSUL/v1/health/service/$SVC?passing=true"

ids() { curl -s -m 3 "$URL" | jq -r '.[].Service.ID // empty' 2>/dev/null | sort; }

ids_before=$(ids)
if [ -z "$ids_before" ]; then
  echo "FAIL  $SVC 当前没有 passing 实例，请先启动服务"
  exit 1
fi

pid=$(pgrep -f "target/release/$PROC" | head -1)
if [ -z "$pid" ]; then
  echo "FAIL  未找到 $PROC 进程"
  exit 1
fi

echo "Consul: $CONSUL  服务: $SVC  模式: $MODE"
echo "kill 前 passing 实例:"; echo "$ids_before" | sed 's/^/  /'

t0=$(date +%s)
case "$MODE" in
  kill9) kill -9 "$pid" ;;
  term)  kill -TERM "$pid" ;;
  *) echo "未知模式: $MODE"; exit 1;;
esac

gone=""
for _ in $(seq 1 $((BUDGET * 10))); do
  now=$(ids)
  gone=$(comm -23 <(echo "$ids_before") <(echo "$now") | head -1)
  [ -n "$gone" ] && break
  sleep 0.2
done

elapsed=$(( $(date +%s) - t0 ))
if [ -z "$gone" ]; then
  echo "FAIL  ${BUDGET}s 内没有实例被摘流（TTL/健康检查未生效）"
  exit 1
fi
echo "PASS  摘流耗时 ${elapsed}s（ID: ${gone}，预算 ${BUDGET}s）"

if [ "$MODE" = "term" ]; then
  # 优雅停机后，TTL 上报任务必须已被停止；否则它会重新注册并把僵尸实例复活
  echo "等待 15s 观察僵尸实例是否复活..."
  sleep 15
  if ids | grep -Fxq "$gone"; then
    echo "FAIL  $gone 被重新注册（TTL 上报任务未随注销停止）"
    exit 1
  fi
  echo "PASS  注销后未复活（TTL 上报任务已停止）"
fi

echo "总预算：摘流 ≤20s + 发现刷新 5s + 调用重试 5s = RTO < 30s"
