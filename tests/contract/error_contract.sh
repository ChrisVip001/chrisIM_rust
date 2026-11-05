#!/usr/bin/env bash
# 错误契约测试：锁定 api-gateway 对外错误语义
#
# 前置：docker 中间件已启动（PG/Redis/Consul），api-gateway 与 relation-service 已运行
# 用法：./tests/contract/error_contract.sh [gateway_addr]
#   gateway_addr 默认 http://127.0.0.1:8000
#
# 锁定的语义：
#   1. 参数缺失 → 400，而不是 500
#   2. 畸形 JSON → 400，而不是静默退化后 500
#   3. 凭据错误 → 401（HTTP 状态与 body.code 一致），不是 500，也不是 200+code:401
#   4. 错误响应统一信封 {code,message,success}，success 恒为 false
#   5. 内部错误细节不外泄（不出现 gRPC Status / 端口 / 源码路径）
set -uo pipefail

GW="${1:-http://127.0.0.1:8000}"
pass=0; fail=0

check() { # name, expected, actual
  if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "PASS  $1";
  else fail=$((fail+1)); echo "FAIL  $1 (期望: $2, 实际: $3)"; fi
}

# 断言一个响应同时满足：HTTP 码、Envelope、success=false、code==HTTP 码、无内部细节
check_error_envelope() { # name, expected_status, body_file, name_prefix
  local name="$1" want="$2" body="$3"
  check "  ├ $name: success=false" "false" "$(jq -r '.success' "$body" 2>/dev/null)"
  check "  ├ $name: body.code == HTTP" "$want" "$(jq -r '.code' "$body" 2>/dev/null)"
  check "  └ $name: 无内部细节" "clean" \
    "$(grep -qE 'status: |details: |\\.rs:[0-9]|50010|50004' "$body" && echo leak || echo clean)"
}

# ── 1. 参数缺失 → 400 ────────────────────────────────────────
code=$(curl -sk -o /tmp/ec_missing.json -w '%{http_code}' -m 5 -X POST \
  "$GW/api/users/registerByUsername" -H "Content-Type: application/json" -d '{}')
check "参数缺失：HTTP 400" "400" "$code"
check_error_envelope "参数缺失" "400" /tmp/ec_missing.json

# ── 2. 畸形 JSON → 400（不能静默退化） ───────────────────────
code=$(curl -sk -o /tmp/ec_badjson.json -w '%{http_code}' -m 5 -X POST \
  "$GW/api/users/registerByUsername" -H "Content-Type: application/json" -d '{"tenantId":')
check "畸形 JSON：HTTP 400" "400" "$code"
check_error_envelope "畸形 JSON" "400" /tmp/ec_badjson.json

# ── 3. 凭据错误 → 401，且 HTTP 与 body.code 一致 ─────────────
code=$(curl -sk -o /tmp/ec_refresh.json -w '%{http_code}' -m 5 -X POST \
  "$GW/api/user/refresh" -H "Content-Type: application/json" -d '{"refresh_token":"not-a-token"}')
check "refresh 无效令牌：HTTP 401" "401" "$code"
check_error_envelope "refresh 无效令牌" "401" /tmp/ec_refresh.json

code=$(curl -sk -o /tmp/ec_login.json -w '%{http_code}' -m 5 -X POST \
  "$GW/api/user/login" -H "Content-Type: application/json" \
  -d '{"username":"__contract_absent__","password":"x","tenant_id":"1","tenant_name":"t","platform":1,"image_code":"1","image_code_key":"1"}')
check "login 用户不存在：HTTP 401" "401" "$code"
check_error_envelope "login 用户不存在" "401" /tmp/ec_login.json

# ── 4. 下游业务错误不能被包装成 500 + 内部细节 ───────────────
code=$(curl -sk -o /tmp/ec_downstream.json -w '%{http_code}' -m 5 -X POST \
  "$GW/api/users/registerByUsername" -H "Content-Type: application/json" \
  -d '{"tenantId":"1","tenantName":"t","username":"","password":"x","serverId":"s1"}')
check "下游校验失败：4xx 或 5xx 都行，但必须是错误信封" "false" \
  "$(jq -r '.success' /tmp/ec_downstream.json 2>/dev/null)"
check "下游校验失败：无内部细节" "clean" \
  "$(grep -qE 'status: |details: |\\.rs:[0-9]' /tmp/ec_downstream.json && echo leak || echo clean)"

# ── 5. 限流 429 也必须是同一信封 ────────────────────────────
for _ in $(seq 1 30); do
  curl -sk -o /tmp/ec_429.json -w '%{http_code}' -m 5 -X POST \
    "$GW/api/users/registerByUsername" -H "Content-Type: application/json" -d '{}' > /tmp/ec_429_code
done
if [ "$(cat /tmp/ec_429_code)" = "429" ]; then
  check "限流 429：用统一信封（有 success 字段）" "false" "$(jq -r '.success' /tmp/ec_429.json 2>/dev/null)"
  check "限流 429：code 与 HTTP 一致" "429" "$(jq -r '.code' /tmp/ec_429.json 2>/dev/null)"
else
  echo "SKIP  限流 429：连续 30 次未触发（可能已有令牌余量）"
fi

echo "──────────────────────────"
echo "错误契约测试: $pass 通过, $fail 失败"
[ "$fail" -eq 0 ]
