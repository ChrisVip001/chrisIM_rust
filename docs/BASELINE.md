# 性能与契约基线（阶段 0 · 验收网）

> 测量日期：2026-11-05 · 环境：本机（macOS，arm64），中间件 docker（PG/Redis/Kafka/Consul）
> 本文件数据是后续所有重构"不得回退"的底线。每阶段结束复测并追加记录。

## 复测方式

```bash
# 1. 中间件
docker compose -f docker-compose.dev.yml up -d && docker start boxim-redis
# 2. 服务（release）
cargo build --release --workspace
./target/release/{relation-service,msg-server,msg-gateway,api-gateway} &
# 3. 契约测试（9 项，必须全绿）
./tests/contract/api_contract.sh
# 4. 网关基线（1 万连接 / 60s / 每连接 1 msg/s）
cargo run --release -p msg-gateway --bin ws_bench -- -- \
    --config ./config/config.yaml --connections 10000 --ramp 500 --duration 60 --rate 1
# 5. 故障注入（人工观测）
./tests/faults/kill-instance.sh <service>
./tests/faults/dependency-down.sh redis 60
```

## 基线 v0（2026-11-05，改造后首次记录）

### 网关接入层（ws_bench，1 万连接 / 60 秒）

| 指标 | 数值 |
|---|---|
| 建连成功率 | 10000 / 10000（失败 0） |
| 建连延迟 | p50=271µs p90=437µs **p99=2.0ms** max=14.3ms |
| 上行吞吐 | **9995 msg/s** 持续 60s（600000 条） |
| 异常断开 | 14（0.14%，疑似压测客户端不回 Pong 触发死链检测，待复核） |

### API 层（/health 200 次串行采样）

| 指标 | 数值 |
|---|---|
| 延迟 | p50=0.5ms p95=0.6ms **p99=1.2ms** max=5.7ms |

### 内存（RSS，空载）

| 服务 | RSS |
|---|---|
| api-gateway | 12MB |
| relation-service | 13MB |
| msg-gateway | 14MB |

### 契约测试

9 项全绿（`tests/contract/api_contract.sh`）：
health 200+version / 业务信封 {code,message,success} / 缺字段 422 可定位 / 未认证 401。

## 与目标的差距（阶段 1-2 待办）

| 目标 | 现状 | 缺口 |
|---|---|---|
| 端到端推送 P99 < 200ms | **未测量** | 需补端到端探针（msg-server 落库→推送链路）；本轮 msg-server 未参与压测 |
| 3000 QPS | 上行 9995 msg/s（接入层）✓ | API 写路径 QPS 未测 |
| 内存 < 512MB/实例 | 空载 12-14MB ✓ | 需加压复测（1 万连接挂载后的 RSS） |
| RTO < 30s | 未测 | 依赖 Consul 健康检查（当前注册 check=None）——阶段 1 关键项 |
| RPO = 0 | 设计满足（Kafka 至少一次+幂等+PG 先行） | 需故障注入实证 |
| 错误率 < 0.1% | 压测异常断开 0.14% | 复核 ws_bench Pong 行为后重新判定 |

## 已由验收网抓出并修复的缺陷

1. **Extension 类型不匹配（运行时 500）**：cache 收窄为领域 trait 后，登录/登出等
   5 个处理器的 `Extension<Arc<dyn Cache>>` 在运行时找不到对应类型——编译期不可见。
   已改为按真实依赖提取 `SessionCache` / `PresenceCache`（登出同时需要两个域）。
   这正是"契约测试先行"的价值：离线测试全绿 ≠ 运行时行为正确。
