#!/usr/bin/env bash
# 对外 HTTP 契约测试：锁定 api-gateway 的可观测行为
#
# 前置：docker 中间件已启动（PG/Redis/Consul），api-gateway 与 relation-service 已运行
# 用法：./tests/contract/api_contract.sh [gateway_addr]
#   gateway_addr 默认 http://127.0.0.1:8000
#
# 契约内容（重构不得改变）：
#   1. /health 返回 200 且含 version 字段
#   2. 业务响应统一信封：{code, message, success} 三键
#   3. 请求体缺字段 → 422，错误信息可定位到字段
#   4. 凭证错误 → 业务错误码，不泄漏内部细节
set -uo pipefail

GW="${1:-http://127.0.0.1:8000}"
pass=0; fail=0

check() { # name, expected, actual
  if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "PASS  $1";
  else fail=$((fail+1)); echo "FAIL  $1 (期望: $2, 实际: $3)"; fi
}

# ── 1. 健康检查 ──────────────────────────────────────────────
code=$(curl -sk -o /tmp/ct_health.json -w '%{http_code}' -m 5 "$GW/health")
check "health 状态码 200" "200" "$code"
check "health 含 version 字段" "true" \
  "$(jq 'has("version")' /tmp/ct_health.json 2>/dev/null)"

# ── 2. 业务响应统一信封 ──────────────────────────────────────
curl -sk -m 5 -X POST "$GW/api/user/login" -H "Content-Type: application/json" \
  -d '{"username":"__contract__","password":"x","tenant_id":"1","tenant_name":"t","platform":1,"image_code":"1","image_code_key":"1"}' \
  -o /tmp/ct_login.json
for key in code message success; do
  check "login 响应含信封键 $key" "true" "$(jq "has(\"$key\")" /tmp/ct_login.json 2>/dev/null)"
done
check "login 信封 success 为 false" "false" "$(jq '.success' /tmp/ct_login.json 2>/dev/null)"

# ── 3. 缺字段 → 422 + 字段可定位 ─────────────────────────────
code=$(curl -sk -o /tmp/ct_422.json -w '%{http_code}' -m 5 -X POST "$GW/api/user/login" \
  -H "Content-Type: application/json" -d '{"username":"x"}')
check "缺字段状态码 422" "422" "$code"
check "422 错误信息含字段名" "yes" \
  "$(grep -qE 'missing field|Failed to deserialize' /tmp/ct_422.json && echo yes || echo no)"

# ── 4. 未认证访问受保护路由 ──────────────────────────────────
code=$(curl -sk -o /tmp/ct_auth.json -w '%{http_code}' -m 5 -X POST "$GW/api/friends/list" \
  -H "Content-Type: application/json" -d '{}')
check "未认证访问受保护路由（拒绝）" "401" "$code"

echo "──────────────────────────"
echo "契约测试: $pass 通过, $fail 失败"
[ "$fail" -eq 0 ]
