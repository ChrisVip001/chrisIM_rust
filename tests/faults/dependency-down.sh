#!/usr/bin/env bash
# 故障注入：依赖不可用（Redis / Kafka / Consul），验证降级行为与恢复
#
# 用法：./tests/faults/dependency-down.sh <redis|kafka|consul> [down_seconds]
#   默认 60s 后自动恢复。脚本只负责断/续依赖并记录观测点，
#   行为断言（fail_open 是否生效、消息是否堆积不丢）由运行日志与验收记录判定。
set -uo pipefail

DEP="${1:?用法: dependency-down.sh <redis|kafka|consul> [down_seconds]}"
SECS="${2:-60}"

case "$DEP" in
  redis)  C=boxim-redis ;;
  kafka)  C=rustim_kafka ;;
  consul) C=rustim_consul ;;
  *) echo "未知依赖: $DEP"; exit 1;;
esac

echo "[$(date +%T)] 停止 $C（${SECS}s 后恢复）"
docker stop "$C" >/dev/null
sleep "$SECS"
echo "[$(date +%T)] 恢复 $C"
docker start "$C" >/dev/null
# 等健康
for _ in $(seq 1 60); do
  st=$(docker inspect -f '{{.State.Health.Status}}' "$C" 2>/dev/null || echo unknown)
  [ "$st" = "healthy" ] && break
  sleep 1
done
echo "[$(date +%T)] $C 已恢复 ($st)"
echo "观测点：各服务日志中的降级/重连记录；Redis 恢复后号段/路由是否自愈"
