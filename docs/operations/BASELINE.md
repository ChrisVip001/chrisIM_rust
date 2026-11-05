# 性能与契约基线（验收网）

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](../README.md)
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
# 满载复测一律走脚本：它会清残留压测进程、等 Kafka 积压归零、校验 JWT_SECRET 一致
JWT_SECRET=<与服务相同的值> ./tests/perf/e2e.sh 500 16 20 10
./target/release/e2e_bench --mode qps --duration 10 --target-qps 3000
# 服务端分段耗时（定位延迟在哪一段）需要被测进程带 IM_STAGE_PROFILE=1 启动
IM_STAGE_PROFILE=1 ./target/release/msg-server
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

## 阶段 2 续：满载 P99 攻关（2026-11-05）

### 先修测量，再修代码

满载复测最大的风险不是调参，而是把「环境漂移」当成「代码退化」。本轮踩到并固化下来的三个坑：

| 漂移源 | 表现 | 处理 |
|---|---|---|
| `JWT_SECRET` 只在服务端存在 | 压测全部 401，参数看起来像 0 成功率 | `tests/perf/e2e.sh` 检查并提示；压测与服务必须同环境变量启动 |
| 上一轮压测进程没退 | CPU 争抢，注入速率对半掉、P99 翻倍 | 脚本先 `pkill` 再测 |
| Kafka 里残留积压 | 测到的是「排空速度」，P99 直接是分钟级 | 脚本等待 `lag=0` 再开始 |

同时新增 `runtime::stage_profile`（`IM_STAGE_PROFILE=1` 开启，默认关闭、关闭时零分配）：
以消息自带的 `create_time` 为基准打三段锚点——等待（Kafka 排队）/ 幂等 / 号段 / 写入 / 推送，
每 5s 输出一次 p50/p90/p99。没有它只能靠猜是哪一段慢。

### 定位结论

分段耗时把「端到端 P99 差两个数量级」拆成了单点：

| 场景 | 等待 | 幂等 | 号段 | 写入 | 推送 | 端到端 P99 |
|---|---|---|---|---|---|---|
| 1000 msg/s | 28ms | 2.8ms | 1.4ms | 5.0ms | 1.5ms | 176ms |
| 3000 msg/s | 5.1s（持续增长） | 5.9ms | 2.7ms | 6.2ms | 3.3ms | 7.5s |

判定实验：把落库临时短路（只保留推送）后，3000 msg/s 端到端 **P99 63ms、到达率 100%** ——
说明 HTTP→Kafka→消费→推送→WS 这段本身能扛 3000/s，**瓶颈唯一在落库**。
同时 `synchronous_commit` 关掉几乎无变化（写入 11.0→6.2ms，P99 7.4→7.5s），
排除「每条消息一次 fsync」的猜测，PG 的开销来自语句与索引本身。

### 本轮落地

| 改动 | 依据 | 代价 |
|---|---|---|
| 攒批窗口 10ms→30ms、排空间隔 2ms→8ms | 3000 msg/s 摊到 4 个消费者后每消费者约 190 msg/s，2ms 窗口只等到 1 条消息，批内并行度退化为 1 | 低速率下最多多等 30ms（仍在 200ms 预算内） |
| 会话水位改为按批汇总回写（`touch_batch`） | 原先每条消息更新一次 `conversation` 行 = 3000 次/秒热点行改写，WAL 与死元组按消息数增长 | 会话列表最后一条消息最多晚一个批次刷新；回写失败只记日志，由下一条消息继续推进 |
| PG 连接池 32→64 | 4 消费者 × 批内并发远超 32 | 连接数上升 |
| `tests/perf/e2e.sh` | 见上表 | — |

1000 msg/s 下 `写入` 由 9.8~13.4ms 降到 5.0ms，P99 由 240ms 降到 176ms。

### 当前能力边界（本机）

| 注入速率 | 结果 |
|---|---|
| 500 msg/s / 16 会话 | P99 120ms ✅（原基线 93ms；窗口自适应后退回短窗口） |
| 1000 msg/s | P99 176ms ✅ |
| 1500 msg/s | 安静窗口下 P99 159ms ✅；同一二进制在宿主繁忙时 P99 413ms（队列稳定在 55ms，尾延迟被宿主抢占放大） |
| 2000 msg/s | P99 389ms（队列稳定在约 150ms，不再增长）❌ |
| 3000 msg/s | P99 7.5s（积压线性增长，实测消费能力约 2250/s）❌ |

**关于可重复性**：本机是「IDE + 浏览器 + Docker Desktop 虚拟机 + 5 个中间件容器 + 4 个服务」
共享 10 核 16GB 的桌面环境。同一条代码路径在不同时段测出的 P99 可以差 2~3 倍
（例如 1500 msg/s 的 159ms 与 413ms），因此：

- 结构性结论（哪一段慢、改哪一段有效）在多次运行中一致，可信；
- 绝对分位数只能在同一时段内横向比较，跨时段必须重跑对照；
- 满载主机采样：容器内 PG 227%、Kafka 100%，折算后宿主 `0.0% idle`。

把消费者并行度从 4 提到 8 反而更差（P99 7.4s→10.7s、到达率 90%），说明是资源饱和而非代码里的锁。

### 环境侧同时修复的问题

- `conversation` 表 75 行活元组累积 9918 个死元组（132 倍膨胀），autovacuum 10 小时未触发：
  默认按比例阈值对小表永远等不到。已在 DDL 里收紧为按行数触发（`scale_factor=0.01`、`threshold=50`）。
- 压测后 `msg_meta` 增长到 60 万行，属正常累积；复测前建议 `VACUUM (ANALYZE)`。

### 下一轮方案（已在下一节实施）

- ~~批量落库~~ → 已实施。
- 次要：**幂等判定并入 Redis 号段脚本**——用同一次往返里的 `SET NX` 判重，
  去掉每条的 PG 预查询；DB 唯一约束仍是最终权威，Redis 丢失时退化为「浪费一个序号」。
- 淘汰：继续加消费者（更差）、关 `synchronous_commit`（无效且牺牲持久性）、
  调大批大小上限（只挪延迟）。

### 门禁

契约 9/9；全 workspace 71 passed / 0 failed / 1 ignored（含新增的攒批与水位回写用例）；
`tests/contract/api_contract.sh` 全绿。本轮未涉及注册/发现/停机路径，故障注入沿用阶段 1 结论。

## 阶段 2 续 · 第二轮：批量落库（2026-11-05）

### 先量化「一条消息到底值多少 CPU」

用 pgbench 绕开业务代码直接压同一台 PG（20 并发、autocommit）：

| 负载 | 语句/事务 | tps |
|---|---|---|
| 普通表 1 条 insert | 1 | 17714 |
| `msg_meta` 分区表 1 条 insert + ON CONFLICT | 1 | 16781 |
| `msg_meta` + `user_conversation` 两行（≈一条消息的落库） | 2 | 10730 |

跑第三项时 PG 吃 443% CPU：`4.43 ÷ 8406 ≈ 0.53ms/事务` ≈ **0.25ms CPU/语句**。
而业务侧 3000 msg/s 时 PG 吃 227%、每条消息 3 条语句：`2.27 ÷ 3000 ÷ 3 ≈ 0.25ms/语句`。

两个数完全同构，结论被钉死：

> **PG 侧 CPU 需求 = 语句总数 × 0.25ms。SQL 没写错，数据库也没到极限——
> 是「每条消息 3 次往返」在 10 核、跑着 Docker 虚拟机的桌面上超预算。**

所以优化的唯一有效方向是**减少语句数**，不是调参。

### 落地

| 改动 | 效果 |
|---|---|
| `save_batch_multi`：整批一条判重（跨会话）+ 一条多行 `INSERT ... ON CONFLICT DO NOTHING RETURNING` | 语句数从「每条消息 2 条」降到「每批 2 条」 |
| 号段按会话分组并发领取（组内串行保序、组间并发） | 号段段 8.1ms → 1.2ms |
| 会话水位回写移到后台 + 限频 200ms | 水位段 100ms → 18µs，退出消费循环 |
| `seq_step` 100 → 1000 | 号段跨界回写从约 30 次/秒降到约 3 次/秒 |
| `process_batch` 重组：解析 → 整批落库 → 按会话并行推送 → 汇总水位 | 推送保序不变（组内串行），落库不再是每组一份 |

失败语义保持：批量落库失败整批退回逐条（一条坏消息不能连坐同批进死信）；
推送失败仍走死信，死信也失败才回退位点。

### 结果（3000 msg/s / 64 会话 / 15s，5 轮）

| 轮次 | P50 | P90 | P99 | 到达率 |
|---|---|---|---|---|
| 1 | 33ms | 47ms | **62ms** ✅ | 100% |
| 2 | 34ms | 52ms | **120ms** ✅ | 100% |
| 3 | 36ms | 58ms | **128ms** ✅ | 100% |
| 4 | 35ms | 55ms | **158ms** ✅ | 100% |
| 5 | 38ms | 96ms | **297ms** ❌ | 100% |

注入口径 2960~3000 msg/s、429 与 5xx 均为 0。优化前同口径是 P99 7.5s、且积压线性增长。

### 数据量是第二个隐形变量

同一份代码，`msg_meta` 从 60 万行涨到 **500 万行（2.8GB）**后，P99 从 100ms 级恶化到
0.7~4s。批次内计时显示恶化落在两段：判重 47~76ms、写入 87ms。

原因：`shared_buffers` 只有 **128MB**（PG 默认值，从未调过），而工作集 2.8GB，
判重的索引探测和插入的索引页都变成物理读。清空压测数据（`TRUNCATE`）后立刻回到
62~158ms，说明是缓存命中率而非算法。

结论：**本机验收必须每轮前确认数据量**，否则会把「索引溢出内存」误判成代码退化。
`docs/sql/init.sql` 未包含 PG 实例级参数，`shared_buffers` 需按机器调整
（生产建议 2~4GB 起步）。

### 剩余尾部

偶发超标的批次，`synchronous_commit=off` 下不出现，属于提交 fsync 的尾部：
`pg_test_fsync` 平均 129µs，但本机 Docker 虚拟盘上偶发 60~100ms。
这是「Docker Desktop 虚拟磁盘 + fsync」的特性，不是 PG 的算法成本，
生产 Linux + 本地 SSD 的 fsync 尾部需在目标机型实测后再下结论。

可选但需要产品拍板：把 `synchronous_commit` 降为 `off`——本机实测 3/3 通过
（180/67/156ms），代价是崩溃可能丢最近几百毫秒的提交，RPO 不再是 0。

### 门禁

契约 9/9；全 workspace 71 passed / 0 failed / 3 ignored（新增跨会话批量写入用例
`batch_write_spans_conversations` 与单会话批量用例，均需本地 PG/Redis）。

## 低风险优化项落地（2026-11-05）

按 `docs/TECHNICAL.md` 9.2 只做了成本/风险为「极低」「低」的四项：注册限流前缀、
批量冲突兜底 + 预校验、熔断半开态探测限流、msg-server / msg-gateway 指标导出。

### 门禁（本轮）

- `cargo test --workspace`：**85 passed / 0 failed / 4 ignored**
  （ignored 全是 `msg-storage/tests/read_spread_e2e.rs` 里需要本地 PG 的用例）。
- `tests/contract/api_contract.sh`：**9/9**。

### 指标导出实测

用 `metrics.enabled: true` 的临时配置各起一份实例（错开端口，避免干扰在跑的服务）：

```bash
curl -s 127.0.0.1:19102/metrics | grep ^ws_
# ws_connections 0 / ws_upstream_total 0 / ws_downstream_total 0
# ws_outbound_dropped_total 0 / ws_dead_conn_total 0
curl -s 127.0.0.1:19101/metrics | grep ^msg_
# msg_consume_batch_total{result="ok"|"fallback"} / msg_inserted_total / msg_duplicate_total
# msg_precheck_rejected_total / msg_push_total{result="success"|"failure"} / msg_dead_letter_total
```

注意这是「零值也在」的验证：计数器与 gauge 在启动时以 0 登记一次，
没有流量时抓取也不返回空响应，否则 Prometheus 告警表达式会因为序列不存在而静默失效。

### 未复测的部分

3000 msg/s / 64 会话的 P99 本轮**没有重跑**：e2e 压测需要 api-gateway、msg-gateway、
msg-server、relation-service 全部按同一份代码重启，本轮只做了单元/契约门禁与指标端点验证。
代码层面这四项都不在消费热路径上（批次级计数器、批量前 O(1) 判断、熔断器内存计数器），
`config.yaml` 的 `metrics.enabled` 仍是 `false`，埋点退化为 noop recorder。
下一轮做性能复测时按上一节的口径（每轮前确认 `msg_meta` 行数、数据量）重跑即可。
