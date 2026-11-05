# 性能与契约基线（验收网）

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
# 5. 故障注入（自动断言）
./tests/faults/rto-consul.sh ws msg-gateway kill9 20   # TTL 过期摘流
./tests/faults/rto-consul.sh chat msg-server term 20   # 优雅停机 + 不复活
./tests/faults/dependency-down.sh redis 20
./tests/faults/dependency-down.sh consul 20
# 6. 全量回归
SQLX_OFFLINE=true cargo test --workspace
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

## 与目标的差距（阶段 2 待办）

| 目标 | 现状 | 缺口 |
|---|---|---|
| 端到端推送 P99 < 200ms | **未测量** | 需补端到端探针（msg-server 落库→推送链路）；本轮 msg-server 未参与压测 |
| 3000 QPS | 上行 9995 msg/s（接入层）✓ | API 写路径 QPS 未测 |
| 内存 < 512MB/实例 | 空载 12-14MB ✓ | 需加压复测（1 万连接挂载后的 RSS） |
| RTO < 30s | **实测达标**（摘流 8s + 发现刷新 5s + 重试 5s = 18s） | 需在真实 k8s 集群复测 Pod 重建耗时 |
| RPO = 0 | 设计满足（Kafka 至少一次+幂等+PG 先行） | 需故障注入实证 |
| 错误率 < 0.1% | 压测异常断开 0.14% | 复核 ws_bench Pong 行为后重新判定 |

## 已由验收网抓出并修复的缺陷

1. **Extension 类型不匹配（运行时 500）**：cache 收窄为领域 trait 后，登录/登出等
   5 个处理器的 `Extension<Arc<dyn Cache>>` 在运行时找不到对应类型——编译期不可见。
   已改为按真实依赖提取 `SessionCache` / `PresenceCache`（登出同时需要两个域）。
   这正是"契约测试先行"的价值：离线测试全绿 ≠ 运行时行为正确。

## 阶段 1 验收记录（2026-11-05，可用性达标）

环境与阶段 0 相同（本机 + docker 中间件），服务为 release 构建。

### RTO 实测（`tests/faults/rto-consul.sh`）

| 场景 | 命令 | 结果 |
|---|---|---|
| kill -9 网关 | `rto-consul.sh ws msg-gateway kill9 20` | **PASS 摘流 8s** |
| 优雅停机网关 | `rto-consul.sh ws msg-gateway term 20` | **PASS 摘流 0s**，15s 内未复活 |
| 优雅停机消息服务 | `rto-consul.sh chat msg-server term 20` | **PASS 摘流 0s**，15s 内未复活 |

摘流 = Consul `?passing=true` 中实例消失。RTO 预算 = 摘流 8s + 发现刷新 5s + 调用重试 5s = 18s < 30s。
`DeregisterCriticalServiceAfter` 最小 1m，只用于清理僵尸注册；摘流靠 TTL 过期后的健康检查过滤，不靠注销计时。

### 依赖故障注入

| 依赖 | 停机 | 观测结果 |
|---|---|---|
| Redis | 20s | 4 个服务注册保持 passing，恢复后契约测试 9/9 |
| Consul | 20s | 停机期间 TTL 上报按 3s 周期快速失败（请求超时 3s），恢复后 4 个服务重新 passing，契约测试 9/9 |

### 门禁

| 门禁 | 结果 |
|---|---|
| 对外 HTTP 契约 | 9/9 全绿（改动后、Redis 恢复后、Consul 恢复后各复测一次） |
| 全 workspace 单测 | 23 个测试目标全部 ok，68 passed / 0 failed（1 ignored） |
| 故障注入自动断言 | 3 项 RTO + 2 项依赖故障，全部 PASS |

### 本阶段抓出并修复的缺陷

2. **TTL 上报任务注销停不掉**：`service_register_center()` 每次新建 `Consul` 实例，注册与注销句柄不在同一实例；停机后任务继续上报甚至重建注册，僵尸实例永不摘流。改为进程级共享句柄表，实测注销日志出现 `Stopped TTL ...` 且 15s 内未复活。
3. **超时被误填为 8080 秒**：`service_center.timeout` 是请求超时秒数，却填了端口号，注册中心不可达时请求会挂起两小时以上。改为 3s，并在代码里限幅 [1,10]。
4. **踩点续命**：TTL 窗口与上报周期同为 15s，一次抖动即误判 critical。窗口收敛为 10s，上报周期取 1/3（3s）。
5. **注册中心重启放大成全链路故障**：发现端拿到空实例集会摘光所有下游。改为保留最后已知良好集合并补单测。
6. **`msg-server` 停机残留注册**：无优雅注销，`chat` 注册项要等 60s 才清理。改为 `serve_with_shutdown` 先注销再退出，main 不再抢先退出，实测摘流 0s。
7. **编排不可运行（k8s 与 prod compose）**：Consul agent 未监听 `0.0.0.0`；注册地址写死 `127.0.0.1`；`msg-gateway` 被探针打 `/health` 却没有该路由；`oss.yaml` / prod compose 的 `oss` 服务部署了不存在的二进制（oss 是库 crate）；compose 健康检查依赖未安装的 `grpc_health_probe`；镜像级 HEALTHCHECK 只适用于 api-gateway。均已修正或删除，注册广告地址由 k8s `status.podIP` / compose 入口脚本按容器 IP 注入。
