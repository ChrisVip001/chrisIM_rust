# RustIM 技术文档：高并发、高可用、高性能的实现与取舍

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](README.md) · 变更记录：`CHANGELOG.md`
>
> 本文回答三个问题：**做了什么**、**为什么这么做**、**效果多大**。
> 所有数字来自 `docs/operations/BASELINE.md` 的本机实测（macOS arm64 / 10 核 16GB / 中间件跑在 Docker）。
> 复测步骤见第 10 节，不要跳过——本机最大的坑是把环境漂移当成代码退化。

## 1. 先修测量，再修代码

性能问题最贵的不是优化，而是**不知道慢在哪**。本项目为此补了三层测量能力：

| 能力 | 工具 | 解决什么问题 |
|---|---|---|
| 接入层 | `msg-gateway --bin ws_bench` | 建连成功率/延迟分位、上行 msg/s、异常断开数。1 万连接、每秒爬坡 500 |
| 端到端 | `e2e_bench --mode e2e` | HTTP 发→Kafka→落库→推送→WS 收，按 `create_time` 算分位数 |
| 吞吐 | `e2e_bench --mode qps` | 目标 QPS 恒定注入，统计 429/5xx 与错误率 |
| 分段归因 | `runtime::stage_profile` | 等待/幂等/号段/写入/推送/端到端/生产 七段各自 p50/p90/p99 |
| 内存 | `tests/perf/rss.sh` | 1 万连接挂载态 RSS，与预算比对 |

`stage_profile` 的设计原则是**可以永久留在热路径上**：

- 关闭时（默认）`record()` 只读一次 `OnceLock` 就返回，不分配不加锁；
- 开启时 1/16 采样、5s 窗口输出并清空，分位数反映当前窗口而不是累计值；
- 各阶段独立采样，所以不能用来做「单条消息的分段占比」对账，只能看趋势。

没有它之前，`P99 = 7.5s` 这种数字只能靠猜；有了它，一次复测就能定位到「等待段持续增长」，
从而判定瓶颈在消费能力而不是推送或网络。

**三个被固化进脚本的漂移源**（`tests/perf/e2e.sh`）：

| 漂移源 | 表现 | 处理 |
|---|---|---|
| `JWT_SECRET` 只在服务端设置 | 全部 401，看起来像 0 成功率 | 脚本校验并提示，压测与服务同环境变量启动 |
| 上一轮压测进程没退 | CPU 争抢，注入速率对半、P99 翻倍 | 先 `pkill` 再测 |
| Kafka 残留积压 | 测到的是排空速度，P99 直接分钟级 | 等 `lag=0` 再开始 |

## 2. 性能模型：语句数决定吞吐

这套系统最有效的一次优化来自一个**可复算的模型**，而不是调参。

用 pgbench 绕开业务代码直接压同一台 PG（20 并发、autocommit）：

| 负载 | 语句/事务 | tps |
|---|---|---|
| 普通表 1 条 insert | 1 | 17714 |
| `msg_meta` 分区表 1 条 insert + `ON CONFLICT` | 1 | 16781 |
| `msg_meta` + `user_conversation`（≈一条消息的落库） | 2 | 10730 |

跑第三项时 PG 吃 443% CPU：`4.43 ÷ 8406 ≈ 0.53ms/事务` ≈ **0.25ms CPU/语句**。
业务侧 3000 msg/s 时 PG 吃 227%，每条消息 3 条语句：`2.27 ÷ 3000 ÷ 3 ≈ 0.25ms/语句`。

两个来源完全同构，于是得到一个可用来做决策的等式：

```
PG 侧 CPU 需求 ≈ 语句总数 × 0.25ms
```

推论：

1. SQL 没写错、数据库也没到极限，是**每条消息 3 次往返**在 10 核桌面上超预算；
2. 唯一有效方向是**减少语句数**，不是调 `shared_buffers`、不是加消费者、不是关 `synchronous_commit`。

两个被实验淘汰的猜测也记录在案：
关掉 `synchronous_commit` 后写入 11.0→6.2ms 但 P99 7.4→7.5s（**排除 fsync 假设**）；
消费者并行度 4→8 反而更差（P99 7.4s→10.7s、到达率 90%，**证明是资源饱和而非锁**）。

## 3. 性能优化清单（逐项：问题 → 做法 → 效果 → 代价）

### 3.1 攒批窗口自适应

- **问题**：最早的攒批只在「没有新消息」时才退出，连续流量下必须攒满 64 条才处理。
  500 msg/s 时单条消息平均白等 64ms。
- **做法**：加时间上限，并按上一批条数自适应：上一批 ≥6 条用 12ms 长窗口，
  否则退回 10ms；每 8ms 收不到新消息就收手（`BATCH_DRAIN_TIMEOUT`）。
- **效果**：P50 117ms → 23ms。
- **代价**：低速率下每条消息固定多等 8~12ms（在 200ms 预算内）。
- 注：早期长窗口取过 30ms（`docs/operations/BASELINE.md` 的阶段 2 记录），后续收敛到 12ms，
  两者不矛盾，是不同轮次的参数。

### 3.2 跨会话批量落库（本轮最大的一刀）

- **问题**：一条消息 2~3 条 SQL，3000 msg/s 就是 6000~9000 语句/秒，
  按 0.25ms/语句 计算约需 1.5~2.2 核，叠加索引与 WAL 后超出单机 PG 的可用份额。
- **做法**：`save_batch_multi` 把整批压成**一条判重 + 一条多行写入**：

```sql
WITH ins AS (
    INSERT INTO msg_meta (conversation_id, seq, server_id, ...)     -- 13 列
    SELECT t.cid, t.seq, t.server_id, ... FROM unnest($1,..., $13) AS t(...)
    ON CONFLICT (conversation_id, server_id) DO NOTHING
    RETURNING conversation_id, server_id, seq
), body AS (
    INSERT INTO msg_content (server_id, content)
    SELECT b.sid, b.content FROM unnest($14, $15) AS b(sid, content)
    WHERE b.sid IN (SELECT server_id FROM ins)      -- 不给没插进去的行留孤儿正文
    ON CONFLICT (server_id) DO NOTHING
), mem AS (
    INSERT INTO user_conversation (user_id, conversation_id)
    SELECT m.uid, m.cid FROM unnest($16, $17) AS m(uid, cid)
    ON CONFLICT DO NOTHING
)
SELECT conversation_id, server_id, seq FROM ins;
```

  用 `unnest` 数组而不是拼 `VALUES` 列表，语句长度与批量大小无关，也能让 PG 按分区键裁剪。

- **效果**：3000 msg/s / 64 会话 / 15s，5 轮 P99 = **62 / 120 / 128 / 158 / 297ms**，
  4/5 达标；优化前同口径是 **7.5s 且积压线性增长**。单实例消费能力从约 2250/s
  提升到 **≥3000/s**（5 轮到达率 100%、结束 lag=0）。
- **代价**：一条坏消息会让整批先付 3 次退避重试再逐条重放；
  批量写是一个事务，失败即整批回滚（见第 6 节的失败语义）。

### 3.3 号段分配与跨界频率

- **问题**：`SELECT ... FOR UPDATE` 自增在热群上是行锁争用；号段方案则把成本转移到
  「跨界回写 PG」，而回写含 fsync，实测偶发 60~100ms。
- **做法**：序号分配放 Redis（一次 Lua 往返，`HINCRBY` + 越界则 `max_seq += step`），
  只有跨界那一条触发 PG 回写（`updated=true`）；`seq_step` 从 100 放大到 1000。
- **效果**：64 会话 / 3000 msg/s 下跨界频率从约 30 次/秒降到约 3 次/秒；
  号段段耗时 8.1ms → 1.2ms。
- **代价**：跳号（未读用 `last_msg_seq` 计算，不受影响）；Redis 丢失后以 PG 水位重建。

### 3.4 会话水位按批回写

- **问题**：每条消息更新一次 `conversation` 行 = 3000 次/秒热点行改写，
  WAL 与死元组按消息数线性增长，还把 autovacuum 拖进热路径。
- **做法**：单条处理只把 `(conversation_id, seq, send_time)` 带出来，
  整批按会话合并成最大值，用一条 `unnest + GREATEST` 批量更新；回写限频 200ms。
- **效果**：水位段 100ms → **18µs**，完全退出消费循环；
  `conversation` 的死元组增长速度下降一个量级。
- **代价**：会话列表最后一条消息最多晚一个批次（200ms）刷新；
  回写失败只记日志不回退位点（否则这批消息会被反复重投，水位永远推不动）。

### 3.5 WebSocket 缓冲与写路径

- **问题**：tungstenite 默认读 128KB + 写 128KB，1 万连接启动即 RSS 1313MB；
  写路径用 `RwLock<SplitSink>`，群推时所有连接争一把锁。
- **做法**：读写缓冲各压到 8KB；写路径改为 per-connection 有界队列（128）+ 唯一写任务；
  群推报文序列化一次，`Bytes` 引用计数共享。
- **效果**：1 万连接挂载态峰值 **159MB**（预算 512MB）；万人群推送内存分配 O(n) → O(1)。
- **代价**：队列满即背压，客户端消费慢会直接体现为发送等待（这是刻意的，比 OOM 好）。

### 3.6 上行并行与保序

- **问题**：上行（客户端→网关→msg-server）最早是单消费者串行，网关侧成为瓶颈。
- **做法**：按 `conversation_id` 哈希到 N 个 worker（默认 8），同会话固定 worker 串行，
  异会话并行；队列容量 256。
- **效果**：基线 v0 在 1 万连接 / 每连接 1 msg/s 下上行 **9995 msg/s** 持续 60s
  （当时异常断开 14，是压测端不回 Pong 触发的假阳性）；修好压测端后阶段 2 复测
  **12631 msg/s、异常断开 0**。
- **代价**：worker 数固定后，热点会话仍然只能串行（保序的必然代价）。

### 3.7 推送定向路由

- **问题**：早期推送靠广播到所有网关，N 个网关就 N 倍放大。
- **做法**：连接注册时写 `user_gateway_route:{user}`（HASH，field=平台），
  推送时先批量查路由表，直连目标网关；路由缺失或投递失败才回退广播。
- **效果**：正常路径只打 1 个网关；失败网关从客户端池剔除并计入 `Partial`，
  由 `needs_broadcast()` 单点判定是否兜底。
- **代价**：路由表是 Redis 里的非权威数据，网关崩溃会留脏条目，靠广播兜底自愈（无 TTL、无清理任务）。

### 3.8 其它小项

| 改动 | 效果 |
|---|---|
| PG 连接池 10（默认）→ 64 | 4 消费者 × 批内并发不再排队等连接 |
| 权限校验缓存化 | 正常路径零 RPC；`validation.fail_open` 可配故障降级 |
| 雪花 ID 临界区只做算术 | 去阻塞，并发唯一性有单测覆盖 |
| 主题副本数自动收敛到 broker 数 | 单 broker 环境不会因 `replication=3` 建主题失败 |
| `conversation` autovacuum 收敛 | 75 行活元组曾累积 9918 死元组（132 倍），默认比例阈值对小表永远不触发 |

## 4. 高并发设计

### 4.1 并发边界在哪

| 层 | 并行单位 | 上限由什么决定 |
|---|---|---|
| 接入 | 每连接异步任务 | 内存（8KB×2/连接）与 fd |
| 上行 | 会话哈希 worker | `websocket.upstream_workers`（默认 8） |
| 传输 | Kafka 分区 | `topic_partitions`（默认 16） |
| 消费 | 消费者任务 | `min(实例数 × parallelism, 分区数)` |
| 落库 | 批量语句 | PG CPU（语句数 × 0.25ms） |
| 推送 | 每会话一个 tokio 任务，组内串行 | 会话数；群推内部并发上限 32 |

**最重要的一条**：消费能力上限是 `min(实例数 × parallelism, 分区数 16)`。
扩到 16 个消费者之后再加实例只会增加 gRPC 接入能力，不会提升消费吞吐 ——
要突破必须加分区或换更大的机器。

### 4.2 保序是怎么保的

三处必须同时成立，缺一条就会乱序：

1. 生产端分区键 = `conversation_id`（同一会话落同一分区）；
2. 消费端按会话归并，**组间并行、组内串行**；
3. 网关侧单写任务，同一连接的帧严格按入队顺序写。

跨会话没有任何顺序保证，这是刻意的设计 —— 否则吞吐会被全局序列化钉死。

## 5. 高可用实现

### 5.1 摘流

| 机制 | 参数 | 为什么是这个值 |
|---|---|---|
| TTL 窗口 | 10s | 15s 时上报也是 15s，一次抖动就误判 critical |
| 上报周期 | TTL/3 = 3s | 踩点续命失效，抖动可容忍一次 |
| `DeregisterCriticalServiceAfter` | 60s | 只清理僵尸注册，不参与摘流判定 |
| 发现刷新 | 5s | 决定 RTO 的第二段 |
| gRPC 调用超时 | 5s | 决定 RTO 的第三段 |

实测：`kill -9` 网关摘流 8s，优雅停机（SIGTERM）摘流 0s，两者合计 RTO ≤ 18s < 30s 目标。

### 5.2 三个必须记住的修复

这些坑都真实发生过，改回去会立刻复发：

1. **TTL 上报任务必须能被注销停掉**。`service_register_center()` 每次新建客户端会导致
   注册与注销不在同一实例，停机的实例继续上报甚至自己重建注册 —— 僵尸永不摘流。
   现在用进程级共享句柄表。
2. **发现端拿到空实例集不能摘光下游**。Consul 重启期间所有服务都会返回空列表，
   直接把所有地址清掉等于放大成全链路故障。现在保留最后已知良好集合 + 2s 追赶。
3. **超时配置要限幅**。`service_center.timeout` 曾把端口号 8080 填进秒数字段，
   注册中心不可达时请求挂起两小时。现在读时 `clamp(1, 10)`。

### 5.3 数据链路的故障语义

| 故障 | 行为 |
|---|---|
| PG 不可用 | 批量落库失败 → 整批重试 3 次 → 逐条重试 → 死信；位点不提交，恢复后重放 |
| Redis 不可用 | 号段失败则落库整体失败（同上）；在线路由失败则推送退化广播；token 校验失败则 401 |
| Kafka 不可用 | 生产端 10s 超时后返回失败，客户端可重试；消费端无消息可消费 |
| Consul 不可用 | 已有连接与调用不受影响（本地保留地址集）；新实例注册失败 |
| 网关崩溃 | Redis 路由残留，定向推送失败 → 广播兜底；客户端重连到其它网关 |

RPO 方面：消息在返回成功前已进 Kafka（`acks=all`），Kafka 侧 `min.insync.replicas=2`；
落库是异步的，进程崩溃丢失的是「已消费未提交位点」的批次，重启后重放，不丢消息。

## 6. 一致性细节：批量写失败会连坐吗

这是批量落库最常被问的问题，答案分两层：

**数据层：是整批回滚。** `insert_batch_multi` 是一条 SQL，三个 CTE 在同一个隐式事务里，
任一错误整条语句回滚，`msg_meta` / `msg_content` / `user_conversation` 都不写。
由于批量是跨会话的，A 用户的消息出问题，同批 B 用户的消息也会一起回滚。

**编排层：不会丢，也不会连带进死信。** 顺序是：

```
整批 SQL 失败 → retry_with_backoff(3 次, 200ms 线性递增)
            → 仍失败 → batchable = false，整批退回逐条 handle_msg
            → 每条独立事务 + 独立重试 3 次
            → 真正坏的那条进 DLQ，其余成功
```

所以代价是「延迟连坐」（整批多付 3 次退避 + 一次逐条重放），不是「数据连坐」。

**什么错会真炸整批**：`ON CONFLICT (conversation_id, server_id)` 只覆盖了一个唯一索引，
撞主键 `(conversation_id, seq)` 会抛 `unique_violation` 整批失败；字段超长/类型非法、
`detail` JSON 构造失败同理。当前 seq 由 Redis `INCR` 分配、且号段只跳号不重号，
所以主键冲突只在号段重建异常时理论上可能出现。

**改进方向**（成本低、收益明确）：把冲突兜底改成无 target 的 `ON CONFLICT DO NOTHING`，
同时覆盖两个唯一索引；再加一层批量前预校验（长度/类型/JSON），
把注定失败的消息提前剔出批量路径，让它自己走逐条 + DLQ。

## 7. 数据层细节

### 7.1 为什么是这个表结构

| 设计 | 理由 | 代价 |
|---|---|---|
| 窄行（不存 avatar/nickname） | 主键扫描时每页放更多行；改名后历史消息不会显示旧昵称 | 拉列表要回用户缓存补全资料 |
| 小正文内联 `inline_content` | 拉列表即得正文，省一次回表 | 行变宽；靠 TOAST 把大值自动外移 |
| 大正文外置 `msg_content` | 图片/文件不拖累元数据扫描 | 读取需一次批量补取（`server_id = ANY($1)`，不是逐条回源） |
| `detail JSONB` 只放取不到的字段 | 能从别的服务查到的都不冗余 | 需要额外的服务调用 |
| HASH 分区，不做时间分区 | 拉取是主键扫描；时间分区会把「拉最近 50 条」变成跨分区 MergeAppend | 删除历史只能按 seq 段迁移 |
| 未读不落库 | 标记已读只写 1 行，与群成员数无关 | 未读是算出来的，必须区分 `last_seq` 与 `last_msg_seq` |

### 7.2 一个必须防的坑：数据量

同一份代码，`msg_meta` 从 60 万行涨到 **500 万行（2.8GB）**后，P99 从 100ms 级恶化到
0.7~4s。原因不是算法，而是 `shared_buffers` 只有 128MB（PG 默认），
判重的索引探测和插入的索引页都变成物理读。清空压测数据后立刻回到 62~158ms。

所以：**本机验收每轮前必须确认数据量**，否则会把「索引溢出内存」误判成代码退化。
`shared_buffers` 是实例级参数，不在 `docs/sql/init.sql` 里，生产建议 2~4GB 起步。

### 7.3 剩余尾部延迟

偶发超标的批次在 `synchronous_commit=off` 下不出现，属于提交 fsync 的尾部：
`pg_test_fsync` 平均 129µs，但 Docker Desktop 虚拟盘上偶发 60~100ms。
这是虚拟磁盘特性，不是 PG 算法成本，需要在本机 SSD 或真实 Linux 上复测才能下结论。

## 8. 实测数据汇总

### 8.1 接入层（1 万连接 / 60s / 每连接 1 msg/s）

| 指标 | 数值 |
|---|---|
| 建连成功率 | 10000 / 10000 |
| 建连延迟 | p50 271µs / p90 437µs / **p99 2.0ms** |
| 上行吞吐 | 9995 msg/s（60s 共 60 万条） |
| 挂载态 RSS | 均值 128MB / 峰值 **159MB** |
| 异常断开 | 0 |

### 8.2 端到端（3000 msg/s / 64 会话 / 15s，5 轮）

| 轮次 | P50 | P90 | P99 | 到达率 |
|---|---|---|---|---|
| 1 | 33ms | 47ms | 62ms | 100% |
| 2 | 34ms | 52ms | 120ms | 100% |
| 3 | 36ms | 58ms | 128ms | 100% |
| 4 | 35ms | 55ms | 158ms | 100% |
| 5 | 38ms | 96ms | 297ms | 100% |

优化前同口径：P99 7.5s 且积压线性增长。5 轮里 4 轮达标，第 5 轮的 297ms 归因于宿主 fsync 尾部。

### 8.3 分段耗时（优化前后）

| 阶段 | 优化前 | 优化后 |
|---|---|---|
| 号段 | 8.1ms | 1.2ms |
| 写入（1000 msg/s） | 9.8~13.4ms | 5.0ms |
| 水位回写 | 100ms | 18µs |
| 端到端 P99（1000 msg/s） | 240ms | 176ms |

### 8.4 容量边界

| 注入速率 | 结果 |
|---|---|
| 500 msg/s / 16 会话 | P99 120ms |
| 1000 msg/s | P99 176ms |
| 1500 msg/s | P99 159ms（宿主繁忙时 413ms） |
| 2000 msg/s | P99 389ms，队列稳定在 150ms 不再增长 |
| 3000 msg/s / 64 会话 | 5 轮 4 次达标（62~158ms） |

注：本机是「IDE + 浏览器 + Docker 虚拟机 + 5 个中间件容器 + 4 个服务」共享 10 核，
同一条代码路径不同时段能差 2~3 倍。结构性结论可信，绝对分位只能同时段横比。

## 9. 容量模型与扩容

### 9.1 三个可直接套用的公式

```
消费能力上限 = min(实例数 × consumer.parallelism, topic_partitions)
PG CPU 需求  ≈ 语句总数 × 0.25ms ≈ (批次数 × 2) × 0.25ms
网关内存     ≈ 连接数 × 16KB + 常驻 ~15MB
```

举例：目标 10000 msg/s，批量 64 条/批 → 约 156 批/秒 → 约 312 语句/秒 → 约 0.08 核 PG。
听起来很轻松，但真实瓶颈往往在「批量能不能攒够」和「分区数够不够」。
按当前参数（16 分区、parallelism 4），4 实例即可跑满 16 个消费者。

### 9.2 按 ROI 排序的优化候选

| 优先级 | 事项 | 预期收益 | 成本/风险 |
|---|---|---|---|
| 高 | 幂等判定并入 Redis 号段脚本（`SET NX` 同往返判重） | 去掉每条消息一次 PG 预查询，判重段从毫秒级降到 Redis 往返 | 中；Redis 丢失退化为浪费一个序号，DB 唯一约束仍是最终权威 |
| 高 | msg-server / msg-gateway 导出 Prometheus 指标 | 消费延迟、死信数、在线数、批次耗时直方图可见 | 低 |
| 高 | 批量冲突兜底改 `ON CONFLICT DO NOTHING` + 批量前预校验 | 消除「一条脏数据拖整批重试」 | 低 |
| 中 | 会话列表上 Redis ZSET | 会话列表从 O(会话数) 次查询降到 1 次 | 中；需要处理双写一致性 |
| 中 | 最近 N 条消息缓存 | 解决主从延迟下「刚发就拉不到」 | 中 |
| 中 | Kafka 分区数 16 → 64 | 消费并行度上限提升 4 倍 | 需要重建主题/迁移 |
| 中 | 跨分区批量提交（减少 commit 次数） | 位点提交开销下降 | 中；需保证回退语义 |
| 低 | 修正注册限流前缀（`/api/user/register` → `/api/users/register*`） | 注册拿到 2 rps 的独立保护，而不是混在 `/api/users` 的 10 rps 里 | 极低 |
| 低 | 心跳死链口径与注释对齐（`missed > max` 实际是第 3 个周期约 90s，配置注释写「约 60 秒」） | 排查断连时不被注释误导 | 极低 |
| 低 | 半开态限并发/成功率 | 熔断恢复更平滑 | 低 |
| 低 | 集群级限流（Redis 计数器） | 多实例限流精确 | 中；热路径多一次 Redis |
| 低 | Redis Cluster 支持 | 单点消除 | 高；号段与路由都要改 |

### 9.3 明确不做的事

- **不追求 exactly-once**：链路上任何一处「只投一次」的保证都无法在故障下验证，
  不如把幂等做扎实。
- **不做全局消息有序**：会退化成单分区串行。
- **不把网关做成消息缓冲**：离线消息落库，上线拉取；网关只负责在线投递。
- **不为省一次往返而 fire-and-forget 推送**：会丢掉推送失败的重试与死信兜底。

## 10. 复测与调优手册

```bash
# 1. 中间件
docker compose -f docker-compose.dev.yml up -d && docker start boxim-redis

# 2. release 构建并起四个服务（JWT_SECRET 必须与压测端一致）
cargo build --release --workspace
JWT_SECRET=<same> ./target/release/relation-service &
JWT_SECRET=<same> IM_STAGE_PROFILE=1 ./target/release/msg-server &
JWT_SECRET=<same> ./target/release/msg-gateway &
JWT_SECRET=<same> ./target/release/api-gateway &

# 3. 契约门禁（9 项必须全绿）
./tests/contract/api_contract.sh

# 4. 端到端（脚本会清残留进程、等 lag=0）
JWT_SECRET=<same> ./tests/perf/e2e.sh 3000 64 15 10

# 5. 接入层与内存
cargo run --release -p msg-gateway --bin ws_bench -- --config ./config/config.yaml \
    --connections 10000 --ramp 500 --duration 60 --rate 1
./tests/perf/rss.sh msg-gateway 55 524288

# 6. 故障与可用性
./tests/faults/rto-consul.sh ws msg-gateway kill9 20
./tests/faults/rto-consul.sh chat msg-server term 20
./tests/faults/dependency-down.sh redis 20
./tests/faults/dependency-down.sh consul 20

# 7. 全量回归
SQLX_OFFLINE=true cargo test --workspace
```

调优时的判断顺序（照这个顺序能少走很多弯路）：

1. 先看 `stage_profile` 的**等待段**是否在增长 —— 增长说明消费跟不上，不是网络问题；
2. 再看**写入段**是否 >10ms —— 看数据量、`shared_buffers`、索引是否溢出内存；
3. 再看**号段段**是否 >3ms —— 看 `seq_step` 与跨界频率；
4. 最后才看推送段 —— 推送慢通常是网关侧背压或路由退化成广播。
