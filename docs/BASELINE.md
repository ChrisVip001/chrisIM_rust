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
# 4b. 1 万连接挂载态 RSS（另开终端，与 ws_bench 并行采样）
./tests/perf/rss.sh msg-gateway 55 524288
# 4c. 端到端延迟 / API 写路径 QPS（需四个服务全部在线）
./target/release/e2e_bench --mode e2e --rate 500 --pairs 16 --duration 20 --settle 10
./target/release/e2e_bench --mode qps --duration 10 --target-qps 3000
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

## 与目标的对账（阶段 2 结束）

| 目标 | 现状 | 缺口 |
|---|---|---|
| 端到端推送 P99 < 200ms | **实测达标 93ms**（500 msg/s、16 会话、lag=0） | 3000 QPS 满载下约 20s；单实例消费吞吐约 1300/s，需批量化或水平扩容 |
| 3000 QPS | **实测达标 3002 QPS**（0 个 429 / 0 个 5xx / 错误率 0） | 限流放到 5000rps；多实例部署需按实际容量重设 |
| 内存 < 512MB/实例 | **实测达标 159MB**（1 万连接挂载态峰值） | 无 |
| 错误率 < 0.1% | **实测达标 0**（1 万连接异常断开 0；HTTP 0 失败） | 无 |
| RTO < 30s | 阶段 1 实测达标（18s 预算） | 需在真实 k8s 集群复测 Pod 重建耗时 |
| RPO = 0 | 设计满足（Kafka 至少一次 + 幂等 + PG 先行） | 仍需故障注入实证（阶段 3） |

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
8. **攒批只在「无新消息」时退出**：连续流量下必须攒满 64 条才处理，500 msg/s 时单条消息平均白等 64ms。加 10ms 时间上限后 P50 117ms→23ms。
9. **连接池与分区数配置漂移**：`PgPool::connect` 默认池 10；`topic_partitions: 16` 实际只有 1 分区，`parallelism: 4` 仅 1 个消费者拿到分区。均改为按配置生效。
10. **每连接 WebSocket 缓冲 256KB**：tungstenite 默认 128KB 读 + 128KB 写，1 万连接 RSS 达 1313MB。改为 8KB×2 后峰值 159MB。
11. **心跳首次检查必然误报**：连接刚建立、Ping 还没发出就记一次「未收到Pong」，1 万连接刷 1 万条 WARN。初始活跃标志修正。
12. **压测端不回 Pong / 收尾期连接重置被计入异常**：`ws_bench` 显式回 Pong，并把停止信号之后的 Close/RST 排除在稳态异常率之外，异常断开 1624→0。

## 阶段 2 验收记录（2026-11-05，性能）

### 新增测量能力

| 工具 | 测什么 | 原先为什么测不了 |
|---|---|---|
| `e2e_bench --mode e2e` | HTTP 写路径 → Kafka → 落库 → 推送 → 接收端 WS 的端到端延迟分位数 | `ws_bench` 上行发 `Heartbeat`，被网关短路，不经过 Kafka/PG/推送；全项目无端到端时间戳 |
| `e2e_bench --mode qps` | 目标 QPS 恒定注入下的成功吞吐、429/5xx、错误率 | `/api/chat` 限流 20rps 会把写路径全挡住 |
| `tests/perf/rss.sh` | 1 万连接挂载态 RSS | 基线只记录空载 RSS（12-14MB） |

口径：`create_time` 由 api-gateway 打点，接收端按 `server_id` 去重后计算 `t_recv - create_time`；
`--pairs` 控制会话对数——消费端同会话串行保序，单会话只能测出「一个会话的上限」。

### 实测结果

| 场景 | 结果 |
|---|---|
| 500 msg/s / 16 会话 / 20s | P50 23ms、P90 36ms、**P99 93ms**、到达率 100%、lag=0 ✅ |
| 3000 QPS 恒定注入 / 10s | **3002 QPS**，0 个 429、0 个 5xx、HTTP 往返 p99 14ms、错误率 0% ✅ |
| 3000 QPS + 64 会话 / 15s | 到达率 100%、结束时 lag=0，但 **P99 19.8s** ❌ |
| 1 万连接挂载 / 60s | 建连 10000/10000、上行 12631 msg/s、**异常断开 0** ✅ |
| 1 万连接 RSS | 均值 128MB、峰值 **159MB** < 512MB ✅ |

门禁：契约 9/9、优雅停机摘流 0s 且不复活、全 workspace 单测 24 个目标全 ok（68 passed / 0 failed / 1 ignored）。

### 未闭合项与下一轮方案

**3000 QPS 满载下的端到端 P99** 未达标：单实例 `msg-server` 可持续消费约 1300/s，每条消息串行经过
「幂等查询 PG → 落库 PG/Redis → 推送 Redis/gRPC」，3000/s 注入必然排队约 12s。

- 首选方案：**批量落库 + 批量推送**（一批一次 SQL / 一次 gRPC），摊薄每条消息的固定网络往返，保持现有投递语义。代价：改动 `MessageWriter`、`Pusher` 接口与 proto，需重跑幂等/DLQ 回归。
- 备选方案：**水平扩容 msg-server**（消费组 3 实例即可覆盖 3000/s），不改业务语义；代价是本机验收需多端口编排，PG/Redis 仍是共享瓶颈。
- 淘汰方案：调大批大小/缩短攒批窗口（只挪延迟，不提升单条处理上限）；推送改 fire-and-forget（少一次往返，但丢掉推送失败的重试与 DLQ 兜底）。

**RPO = 0** 仍缺故障注入实证，留待阶段 3 与结构门禁一起做。
