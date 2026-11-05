# RustIM 架构设计

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](README.md)
>
> 本文描述系统**当前实现**的结构与取舍。凡是「设计意图已写但代码未落地」的地方都单独标注，
> 见 [第 9 节](#9-已知落差注释与实现不一致的地方)。性能数据来自 `docs/operations/BASELINE.md` 的本机实测。

## 1. 设计目标与硬约束

RustIM 是一个用 Rust 实现的即时通讯后端。所有架构决策都可以回溯到下面五条硬约束：

| 约束 | 含义 | 违反后果 |
|---|---|---|
| **落库是唯一真相** | 消息以 PostgreSQL 为准，Redis/Kafka 都只是加速与传输 | 任何「靠内存状态恢复消息」的设计都不可接受 |
| **至少一次投递 + 幂等消费** | Kafka 位点手动提交，允许重复投递，用 `server_id` 唯一约束兜底 | 追求 exactly-once 会把复杂度堆在链路中间且无法验证 |
| **会话内严格保序，跨会话无序** | 分区键、落库序号、推送并发都以会话为界 | 全局有序会退化成单分区串行，吞吐被钉死 |
| **一条消息只写一次**（读扩散） | 不存在 user 维度的消息副本 | 写扩散在万人大群里是 10001 行写入 |
| **不新增中间件类型** | 只用 PG / Redis / Kafka / Consul 四件基础设施 | 每多一种存储就多一套运维与故障域 |

一个明确的现实前提：**所有性能数字都在一台 10 核 16GB 的 macOS 桌面上测得**，
5 个中间件容器 + 4 个服务共享宿主 CPU。因此结构性结论（哪一段慢、改哪一段有效）
可以跨环境复现，绝对分位数只能在同时段内横向比较。

## 2. 系统全景

### 2.1 进程与端口

workspace 有 9 个 crate，但**只有 4 个是可部署进程**，其余都是库：

| 进程 | 协议 / 端口 | 职责 |
|---|---|---|
| `api-gateway` | HTTP `8000`（编排层对外映射 8080） | 统一入口、JWT 认证、路由转发、限流、熔断、文件签名 |
| `msg-gateway` | WebSocket `8085` + gRPC `50001` | 长连接接入、连接注册表、消息下行投递 |
| `msg-server` | gRPC `50010` + Kafka 消费者 | 消息生产/消费/落库编排、推送路由 |
| `relation-service` | gRPC `50004` | 用户 / 好友 / 群组（模块化单体，一个进程一个端口） |

| 库 crate | 被谁依赖 | 内容 |
|---|---|---|
| `contract` | 全部 | proto 定义、领域类型、统一错误、会话键推导 |
| `runtime` | 全部 | 配置加载、日志、服务注册发现、gRPC 客户端、优雅停机、雪花 ID、分段计时 |
| `cache` | 消息/关系服务 | Redis 领域封装（号段、会话、关系、在线、未读），5 个窄 trait |
| `msg-storage` | `msg-server` | 读扩散存储实现（PG）、批量写入、幂等、水位 |
| `oss` | `api-gateway` | S3 / COS / OSS 统一抽象 + 预签名直传 |

### 2.2 部署拓扑

```
                    ┌──────────┐   ┌──────────┐   ┌──────────┐
                    │ Web      │   │ Mobile   │   │ Desktop  │
                    └────┬─────┘   └────┬─────┘   └────┬─────┘
                         │ HTTP          │ WS           │
                    ┌────▼───────────────▼──────────────▼─────┐
                    │              nginx / LB                  │
                    └────┬─────────────────────────┬──────────┘
                         │                         │
              ┌──────────▼─────────┐    ┌──────────▼───────────┐
              │   api-gateway      │    │    msg-gateway       │
              │ 认证 限流 熔断 路由 │    │ WS 接入 + 连接表      │
              └───┬────────────┬───┘    └───┬──────────────┬───┘
                  │ gRPC       │ gRPC       │ gRPC 上行     │ gRPC 下行
                  │            │            │              │
        ┌─────────▼──────┐  ┌──▼────────────▼───┐   ┌───────▼────────┐
        │relation-service│  │    msg-server     │   │  回推其他网关   │
        │ 用户/好友/群组  │  │ 生产 + 消费 + 推送 │   └────────────────┘
        └────────┬───────┘  └───┬───────────┬───┘
                 │              │           │
              ┌──▼───┐      ┌───▼────┐  ┌───▼────┐
              │  PG  │      │ Kafka  │  │ Redis  │
              └──────┘      └────────┘  └────────┘
```

图中省略了「msg-server 经 msg-storage 写 PG」这条连线。依赖用途：
PG = 会话/消息/关系数据，Kafka = 消息传输与削峰，Redis = 号段/在线路由/未读/token，
Consul = 服务注册与发现（全部服务共用）。

服务发现名字只有三个常量：`relation`、`chat`、`ws`。api-gateway 对 user/friend/group
三个客户端**全部指向 `relation`** —— 内部拆几个模块对外不可见，调用方不需要知道。

### 2.3 消息主链路（写路径）

一条单聊消息从发出到对端收到，经过 8 跳：

```
1. 客户端 POST /api/chat/send
2. api-gateway    CORS → 体积限制 → 超时 → 访问日志 → 指标 → 限流 → 熔断 → JWT 校验 → gRPC 转发（打 create_time）
3. msg-server     权限/关系校验（缓存化，正常路径零 RPC）
                  → 生成 server_id(nanoid) → 按 conversation_id 作分区键写 Kafka
4. Kafka          rustIM-chat，16 分区，acks=all + enable.idempotence
5. msg-server     消费组 chat 内 N 个消费者，按分区均衡
                  攒批（≤64 条 / 10~12ms）→ 按会话归并 → 整批一次落库
6. msg-storage    Redis 领号段 → 一条 CTE 写入 msg_meta/msg_content/user_conversation
7. msg-server     按会话并发推送：查 Redis 路由表 → 直连目标网关 gRPC
8. msg-gateway    查进程内连接表 → 有界队列 → 单写任务 → WS 帧
```

关键点：**第 6 步之前不分配序号**。序号在消费端按会话统一分配，
发送端预分配会在重试/乱序时产生与落库顺序不一致的号。

上行（客户端发消息）与下行（服务端推消息）是两条独立路径：
上行走 WS → msg-gateway → `ChatService.SendMsg` → Kafka；
下行走 msg-server → `MsgService.SendMessage` → msg-gateway → WS。
两者共享 `conversation_id` 的推导规则，所以分区、序号、网关保序三者天然对齐。

## 3. 服务详解

### 3.1 api-gateway

**中间件链**（外 → 内，即代码里 `layer()` 的逆序）：

```
CORS → RequestBodyLimit(10MB) → Timeout(30s) → RequestLogger → Metrics
     → RateLimit → CircuitBreaker → JWT 认证 → 转发
```

日志与指标在限流/超时之外：429、413、408 与熔断短路响应都会进指标和访问日志，
否则「最需要观测的失败恰好不可见」。

- **路由 = 静态路由 + 配置驱动**。`/health`、`/metrics`、登录/刷新是静态注册的，
  业务前缀（`/api/users`、`/api/friends`、`/api/groups`、`/api/chat`、`/api/common`）
  来自 `config.yaml` 的 `gateway.routes`，每条前缀同时注册精确路径与通配路径；
  未匹配路径统一 404 + 信封，不再是 axum 的空 body 404。
- **错误语义**：所有失败走 `GatewayError`，`{code,message,success:false}` 且
  **HTTP 状态码 == `code`**。tonic 状态码映射成 400/401/403/404/409/429/502/503/504，
  内部细节（gRPC Status 原文、连接串）只进日志。参数错误不会再返回 500。
- **限流 = governor GCRA**（令牌桶的等价实现），三层检查：全局 → 路径 → IP，
  命中任一即拒绝**且不再扣减后两层**的配额。参数：全局 5000 rps / burst 500，
  `/api/users` 10 rps，`/api/chat` 5000 rps，登录 5 rps / burst 3，注册 2 rps。
  前缀匹配按路径段边界（`/api/user/loginXXX` 不再被当成 `/api/user/login`）。
  429 也返回统一信封，并带 `Retry-After`（亚秒级等待向上取整到 1）。
  **这是进程内限流**，多实例部署时总量按实例数放大。
- **熔断 = 三态计数器**（Closed / Open / HalfOpen），连续 5 次失败转 Open，30s 后转 HalfOpen。
  只有 5xx 与传输错误计失败，4xx 不触发。
  分桶键是**真实下游服务名**（`relation` / `chat`，与 `service_names` 一致），且只覆盖
  真正转发的路径（`/api/users`、`/api/friends`、`/api/groups` → `relation`；`/api/chat` → `chat`）。
  网关自己处理的路由（`/health`、`/metrics`、登录/刷新/登出、上传签名）不参与熔断，
  免得网关自身的 5xx 反过来把登录也熔断掉。`gateway.circuit_breaker.enabled` 为 false 时整体透传。
- **认证 = IP 黑名单 → 路径白名单 → JWT 验签 → Redis 会话比对**。
  验签通过后还要与 Redis 中该平台的 token 比对，这让「同平台重新登录踢掉旧 token」
  和「多端各自独立」都成立，代价是每次请求一次 Redis 读。令牌原文**永不进日志**
  （只记长度与指纹）。
- **客户端 IP**：`api_utils::net` 统一口径 —— 对端地址是权威，只有对端在
  `gateway.auth.trusted_proxies` 里时才采信 `X-Forwarded-For`（默认空 = 忽略 XFF）。
  日志/限流/黑名单三处共用，不再各写一套。
- **gRPC 客户端池**：启动预热 + 按需重建，**只重建缺失的那一个**，失败进入指数退避
  （1s→30s）。此前后端宕机时每个请求都会重建全部 4 条连接 = 每请求 4 次 Consul 查询，
  且每次重建会泄漏一个永久轮询 Consul 的发现任务（`discovery.run()` 现在在接收端关闭时退出）。

### 3.2 msg-gateway（长连接网关）

网关是唯一持有连接状态的服务，所有设计围绕「1 万连接只花 159MB」：

| 设计 | 做法 | 为什么 |
|---|---|---|
| 连接表 | `DashMap<UserID, DashMap<Platform, Conn>>` | 外层用户、内层平台，天然支持多端在线，无全局锁 |
| 写路径 | per-connection 有界队列（128）+ **唯一写任务** | 消除 `RwLock<SplitSink>` 争用；队列满即背压，不无限堆内存 |
| 缓冲区 | 读/写各 8KB（tungstenite 默认 128KB×2） | 1 万连接下默认值实测 RSS 1313MB → 159MB |
| 上行并行 | 按会话哈希到 8 个 worker，同会话固定 worker | 异会话并行、同会话保序，`upstream_workers` 可配 |
| 群推序列化 | 报文序列化一次，`Bytes` 引用计数共享 | 万人群内存分配 O(n) → O(1) |
| 死链检测 | 30s Ping；`missed > heartbeat_max_missed(2)` 即第 3 个周期（≈90s）断开 | 只清假在线，不误杀活跃连接 |
| 重复登录 | 同用户同平台新连接覆盖旧连接，旧连接收 Close 4001 | 避免一台设备双活导致消息乱序 |

**跨节点投递**靠 Redis：连接注册时写 `user_gateway_route:{user}`（HASH，field=平台，
value=网关 gRPC 地址），注销时显式删除。这个 key **没有 TTL** ——
网关崩溃留下的残留由「定向投递失败后广播兜底」自愈，省掉后台清理任务。

**离线不排队**：用户没有本地连接时不投递，因为消息已经落库，上线后按 `seq` 游标拉历史。
这是「不把网关做成消息缓冲区」的必然结果。

### 3.3 msg-server

单进程内三类任务：1 个生产者侧 gRPC 服务 + N 个消费者任务 + 1 个推送器。

**消费循环**（每实例 N 个，N = `parallelism`，默认 4）：

```
阻塞等首条 → 在 deadline 内尽可能多收（每 8ms 静默即收手）
→ 按 conversation_id 归并 → 并发查一次会话成员（组间并行）
→ 整批一条 SQL 落库（失败退回逐条）
→ 按会话并发推送（组内串行保序）
→ 汇总水位，200ms 限频回写
→ 按分区提交最大位点
```

攒批窗口是自适应的：上一批 ≥6 条说明满载，用 12ms 长窗口换批内并行度；
否则退回 10ms，避免低速率下平白拉高尾延迟。

**失败处理三级**：

1. 单条处理失败 → 线性退避重试 3 次（200ms 递增）
2. 仍失败 → 原样投递 `<topic>.dlq`（分区数与主主题对齐，便于重放）
3. 死信也写不进去 → 该分区 `seek` 回退，等待重新投递；已处理的靠幂等闸门跳过

批量路径的失败语义单独处理：整批 SQL 失败先整批重试，再失败则**退回逐条**，
保证一条坏消息不会把同批其他会话的消息一起拖进死信。

### 3.4 relation-service：为什么是模块化单体

用户、好友、群组三个域拆成进程会带来三个纯损耗：三次注册、三次跨进程调用、
三份连接池。而它们的写路径互不重叠、读路径互相依赖（好友列表要用户信息）。

所以合并为一个进程、一个端口、一次注册，三个 gRPC 服务挂在同一个 `Server` 上，
共享一个 PG 连接池，域之间**进程内直调**（构造期注入，依赖单向：
`UserConfigQuery ← FriendServiceImpl ← UserServiceImpl ← GroupServiceImpl`）。
对外仍然是一个服务名 `relation`，网关侧无需感知内部拆分。

## 4. 数据模型

### 4.1 四张表

| 表 | 主键 | 作用 | 关键取舍 |
|---|---|---|---|
| `conversation` | `conversation_id` | 会话级水位：`last_seq`（号段已预留上界）、`last_msg_seq`（真实最后一条）、`last_msg_time` | 号段会跳号，所以「未读基准」和「号段水位」必须是两列 |
| `msg_meta` | `(conversation_id, seq)` | 唯一消息级表，窄行 | 主键即查询路径；不存 avatar/nickname（从缓存取，避免改名后历史消息显示旧值） |
| `msg_content` | `server_id` | 大正文外置（图片/文件/长文本） | 小正文内联在 `msg_meta.inline_content`（默认 ≤1024B，走 TOAST），省一次回表 |
| `user_conversation` | `(user_id, conversation_id)` | 已读游标、置顶、免打扰 | 未读数**不落库**，用 `last_msg_seq - last_read_seq` 现算，标记已读只写 1 行 |

分区：`msg_meta` 32 个 HASH 分区、`msg_content` 16 个、`user_conversation` 32 个，
**刻意不做时间二级分区**——拉取路径是主键扫描，加时间分区会让「拉最近 50 条」
退化成跨 12 个月分区的 MergeAppend，得不偿失。归档按 seq 段迁移，不做定期 DELETE。

索引只有三个，都是补维度：`(conversation_id, send_time DESC)`、
`(last_msg_time DESC)`（会话列表冷路径）、`(user_id)`（用户维度）。

### 4.2 读扩散：一条消息只写一次

万人群发一条消息：**2 行写入**（`msg_meta` + 大正文时的 `msg_content`）。
写扩散方案是 10001 行。读取时按 `(conversation_id, seq)` 主键范围扫描，
未读用游标相减。

代价是「已读」不能按消息行标记，只能推进游标 —— 这恰好也是 IM 的语义
（跨设备已读同步只需同步一个数字）。

### 4.3 序号分配：Redis 号段

```
incr_conversation_seq（一次 Lua 往返，原子）
  ├─ 键不存在 → 返回 0，走冷启动：读 PG 水位 → init → 立即回写预留上界 → 重试
  ├─ cur_seq <= max_seq → cur_seq+1 直接返回
  └─ cur_seq > max_seq → max_seq += step，返回 updated=true → 异步回写 PG
```

- **步长代码默认 5000，`config.yaml` 配 1000，`config-prod.yaml` 样例是 10000**。跨界回写是一次独立 PG 事务（含 fsync），
  步长 100 时 3000 msg/s 下每秒跨界约 30 次，实测这些 UPDATE 偶发 60~100ms，
  会直接把该会话的消费卡住；放大到 1000 后降到约 3 次/秒。
- **冷启动立即回写**：新号段的预留上界必须马上落 PG，
  否则 Redis 丢失后会重新分配已经用过的序号。
- **宁可跳号，不可重号**：Redis 数据丢失时以 PG 的 `last_seq` 为基线重新开号，
  代价是空洞，收益是唯一性。未读用 `last_msg_seq` 计算，跳号不影响业务。
- 为什么不用 `SELECT ... FOR UPDATE` 或 PG sequence：热群会在一行上串行化，
  号段把分配成本从「一次行锁」降到「一次 Redis 往返 / 1000 条」。

### 4.4 幂等：三道闸

| 闸 | 位置 | 作用 |
|---|---|---|
| 预查询 | 落库前 `find_existing` / `find_existing_multi` | 重复投递**不消耗号段、不推高水位**，直接返回已有序号 |
| 唯一约束 | `uk_msg_meta_server (conversation_id, server_id)` + `ON CONFLICT DO NOTHING` | 并发抢先的最终权威，返回行集合即「谁真的插进去了」 |
| 推送判定 | 只有 `inserted = true` 才推送 | 重放不重复推送，位点正常推进 |

`msg_content` 的写入用 `WHERE server_id IN (SELECT server_id FROM ins)` 限定，
并发下没插进去的行不会留下孤儿正文。

## 5. 一致性与投递语义

| 维度 | 保证 | 边界 |
|---|---|---|
| 投递 | at-least-once | 位点手动提交，重启/回退会重投，由幂等兜底 |
| 顺序 | **会话内**严格有序（分区键 + 消费端按会话串行 + 网关单写任务） | 跨会话不保证；同一会话的多端各自按 seq 排序 |
| 持久性 | 消息进 PG 才算成功；Kafka 侧 `acks=all` + 幂等生产者 | 服务端返回成功时消息已进 Kafka，未必已落库（几十毫秒窗口） |
| 撤回 | 同步 UPDATE + 一条通知消息 | 见 [PRODUCT.md 第 6 节](PRODUCT.md#6-已知行为与坑) |
| 重复 | 不会产生重复行、不会重复推送 | 会重复消费（可接受，成本是一次预查询） |

位点提交的取舍：整批里只要有一条消息既没落库也没进死信，该分区就不提交并 `seek` 回退。
已在 PG 里的消息会被重新投递一次，然后在预查询处被拦下 —— 用「多一次查询」换「不丢消息」。

## 6. 高可用

### 6.1 摘流链路

```
SIGTERM/SIGINT → runtime::shutdown 统一信号
  → 先从 Consul 注销（摘流 0s，实测）
  → 停止接收新请求 / 关闭 gRPC serve
  → 排空存量（api-gateway 30s 排空；WS 连接收到关闭帧）
  → 停止 TTL 上报任务（否则僵尸实例会自己续命）
  → 进程退出
```

`msg-gateway` 的顺序特殊：**先摘 gRPC 再摘 WS**。反过来会出现
「网关已不接新连接，但 msg-server 还在往里推」的窗口。

### 6.2 TTL 健康检查

- TTL 10s，上报周期 **TTL/3 = 3s**。踩点续命（窗口 15s、上报 15s）一次抖动就误判 critical。
- `DeregisterCriticalServiceAfter` 最小 1 分钟，只用于清理僵尸注册；
  **摘流靠 `passing=true` 过滤，不靠注销计时**。
- 上报任务每轮先 `GET /v1/agent/service/{id}` 核查注册是否还在，
  Consul 重启丢状态时用原始载荷重建并立即 pass —— 这是「注册中心重启放大成全链路故障」的修复点。
- **发现端拿到空实例集不摘光**：保留最后已知良好集合，并以 2s 短间隔追赶。
  否则 Consul 抖动一次就会把所有下游清空。

### 6.3 RTO 预算

| 环节 | 耗时 |
|---|---|
| TTL 过期摘流 | ≤ 10s（kill -9 实测 8s；优雅停机 0s） |
| 发现刷新 | ≤ 5s |
| 调用重试 | ≤ 5s |
| **合计** | **18s < 30s 目标** |

k8s 侧配套：readiness 5s×2、liveness 15s×3、`terminationGracePeriodSeconds=30`、
`maxUnavailable: 0`（msg-gateway）、PDB `minAvailable: 1`。

### 6.4 当前单点

PG、Redis、Consul 在本仓库基线里都是单实例。业务层已经做到无状态可横向扩，
但这三个依赖仍是故障域。生产建议见 [TECHNICAL.md](TECHNICAL.md#9-容量模型与扩容)。

## 7. 关键取舍总表

| 决策 | 放弃了什么 | 得到了什么 | 代价 |
|---|---|---|---|
| 读扩散存储 | 按用户维度直接读消息副本 | 群消息写入与群规模无关（2 行 / 万人群） | 拉取要按会话扫描 + 客户端自己维护游标 |
| Kafka 而非直接写 PG | 更低的端到端延迟（省一次 broker 往返） | 削峰、可重放、生产端与落库解耦 | 端到端多 10~20ms；需要处理重复投递 |
| 分区键 = 会话 ID | 全局有序 | 会话内有序 + 16 分区并行 | 热点会话会集中到一个分区 |
| 序号在消费端分配 | 发送端可立即返回 seq | 序号与落库顺序严格一致 | 客户端要等推送才知道 seq |
| 批量落库（一条 CTE） | 单条失败隔离 | 语句数从「每条 2~3 条」降到「每批 2 条」，P99 7.5s → 百毫秒 | 一条坏消息让整批先付一次重试+逐条重放 |
| Redis 号段而非 PG 自增 | 号段绝对连续 | 热群不再有行锁争用，领号一次 Redis 往返 | 跳号；Redis 丢失要重建 |
| 水位按批 + 200ms 限频 | 会话列表即时刷新 | 水位段 100ms → 18µs，退出消费热路径 | 列表最后一条最多晚一个批次 |
| 网关在线表放内存 | 网关无状态 | 推送路径零额外查询，1 万连接 159MB | 扩缩容时连接要重连迁移 |
| 定向推送 + 广播兜底 | 纯广播的简单 | 正常路径只打目标网关；失败才广播 | 路由表脏数据会退化成广播 |
| 模块化单体（relation） | 独立扩缩容 / 故障隔离 | 少两次 RPC、少两份连接池、一次注册 | 一个域出问题三个域一起受影响 |
| 进程内限流 | 集群级精确限流 | 零额外依赖、零网络开销 | 多实例时总量按实例数放大 |

## 8. 可观测性

| 能力 | 现状 |
|---|---|
| 日志 | `tracing`，JSON/文本可切，按天切割保留 7 天，`RUST_LOG` 覆盖配置；gRPC 拦截器把 `x-trace-id`、`caller` 写进日志 |
| 指标（api-gateway） | router 内 `/metrics`（与服务同端口，由 `gateway.metrics_endpoint` 指定）：`gateway_requests_total`（method/path/service）、`gateway_responses_total` 与 `gateway_errors_total`（method/path/service/status）、`gateway_request_duration_seconds`（service）。`path` 标签已归一化（`/api/users/{id}/*`），`service` 用真实下游名（`relation`/`chat`/`gateway`） |
| 指标（msg-server） | 独立监听 `:9101/metrics`（`metrics.enabled` 控制，`metrics.port` 覆盖）：`msg_consume_batch_total{result}`、`msg_consume_batch_size`、`msg_consume_batch_duration_seconds`、`msg_inserted_total`、`msg_duplicate_total`、`msg_precheck_rejected_total`、`msg_push_total{result}`、`msg_dead_letter_total` |
| 指标（msg-gateway） | 独立监听 `:9102/metrics`：`ws_connections`、`ws_upstream_total`、`ws_downstream_total`、`ws_outbound_dropped_total`、`ws_dead_conn_total` |
| 链路 | OTLP 导出走 `telemetry` feature，默认关闭（开启会引入旧 tonic 链，见 `docs/operations/TELEMETRY.md`） |
| 分段耗时 | `runtime::stage_profile`，`IM_STAGE_PROFILE=1` 开启，1/16 采样，5s 输出 p50/p90/p99（等待/幂等/号段/写入/推送/端到端/生产） |
| 慢路径告警 | 单批落库 >100ms、批次总耗时 >200ms 各打一条带分段明细的 warn |

指标语义与排查口径见 [TECHNICAL.md 9.2.1](TECHNICAL.md#921-指标导出已实施)。
两个消息服务用 `runtime::metrics_server` 起监听（`PrometheusBuilder::build()` + 手工 spawn exporter，
不能用只装 recorder 的 `install_recorder()`，否则端口永远连不上）。


## 9. 已知落差（注释与实现不一致的地方）

写在这里是为了避免后来者把注释当事实：

| 声称 | 现状 | 影响 |
|---|---|---|
| 会话列表热数据走 Redis ZSET | 无任何 ZSET 代码，实际是 DB 查询 + 内存排序 | 会话列表在会话多时是 O(会话数) 次查询 |
| Redis 最近 N 条消息缓存（`hot_cache_messages`） | 只有配置项，无读写实现 | 主从延迟时「刚发就拉」可能拉不到 |
| `rpc.pusher` / 端口 50003 | 不存在。推送器是 msg-server 进程内模块 | 文档/配置会误导部署 |
| `server.connection.max_connections: 10000` | 代码无引用点，没有硬上限 | 实际约束是内存与文件描述符 |
| api-gateway 端口 | `config.yaml` 是 8000，k8s 清单写 8080（需 ConfigMap 覆盖） | 直接 apply k8s 会探针失败 |
| msg-server gRPC 端口 | `config.yaml`/compose/k8s 是 50010，`config-prod.yaml` 是 50002 | 按生产样例部署时端口与文档不一致 |
| 图片验证码 | 代码里是一段被注释掉的校验（请求字段也删了），`path_whitelist` 里仍列着 `genCaptchaImage` | 登录目前只靠密码 + 限流，爆破防护弱于文档给人的印象 |
| 分布式锁 | cache crate 无锁实现 | 需要时得自己加（如群成员变更的互斥） |
| Redis Cluster | 单 `Client::open(url)`，无 cluster/sentinel 字段 | 只能单实例或前置代理 |

## 10. 架构演进方向

按「投入产出比」排序，详细论证见 [TECHNICAL.md 第 9 节](TECHNICAL.md#9-容量模型与扩容)：

1. **幂等判定并入号段脚本**：用同一次 Redis 往返里的 `SET NX` 判重，去掉每条消息的 PG 预查询。
2. **会话列表上 Redis ZSET**：把上面那条「注释先于实现」补齐，会话列表从 N 次查询降到 1 次。
3. ~~**可观测性补齐**：msg-server / msg-gateway 导出 Prometheus 指标~~（已完成，见第 8 节）。
4. **读扩散的读路径优化**：最近消息缓存（Redis 或本地 LRU），解决主从延迟与冷启动回表。
5. **多租户与存储分片**：`tenant_id` 已经出现在对象存储路径里，但库表还没有租户维度。
