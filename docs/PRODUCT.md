# RustIM 产品文档

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](README.md) · 变更记录：`CHANGELOG.md`
>
> 面向三类读者：要接入的客户端开发者、要评估这套系统能不能用的技术负责人、
> 要在此基础上做二次开发的团队。架构与实现细节见 [ARCHITECTURE.md](ARCHITECTURE.md)、
> [TECHNICAL.md](TECHNICAL.md)。

## 1. 产品定位

**一套可自托管、可水平扩展的 IM 后端基座**。它提供「账号—关系—会话—消息—推送」这条主干的
完整实现，以及配套的部署、观测与验收工具；不提供 UI、不绑定客户端形态。

适合的场景：

- 需要在自有基础设施上跑 IM，数据不出内网；
- 需要一个能扛住万级长连接、千级消息写入的后端起点，而不是从零搭 WebSocket 网关；
- 想学习「读扩散存储 + Kafka 异步落库 + 长连接网关」这套工程组合怎么落地。

不适合的场景（明确表态）：

- 开箱即用的 SaaS 即时通讯（多租户、计费、审计都还没有）；
- 音视频通话（当前只有信令通道的雏形，没有媒体面）；
- 千万级 DAU 的单集群（Redis、PG、Consul 目前都是单实例基线，需要先做分片）。

## 2. 能力矩阵

成熟度：**稳定** = 有契约测试或单测覆盖、压测验证过；**可用** = 功能完整但缺少自动化验收；
**雏形** = 接口存在、实现不完整。

### 2.1 账号与用户

| 能力 | 成熟度 | 说明 |
|---|---|---|
| 用户名注册 / 手机号注册 | 稳定 | 密码 bcrypt 存储；限流走 `/api/users` 10 rps（见 4.1 的已知缺陷） |
| 密码登录 / 手机验证码登录 | 稳定 | 登录限流 5 rps，防撞库 |
| 多端登录（Web / iOS / Android / Desktop） | 稳定 | 同平台重复登录踢旧连接，跨平台并存 |
| JWT 刷新与「同平台单 token」 | 稳定 | token 存 Redis，登出即失效 |
| 找回密码 / 更换手机号 / 注销账号 | 可用 | 依赖短信服务配置 |
| 用户资料、用户配置、按 ID/用户名搜索 | 可用 | |
| 图形验证码 | 可用 | |

### 2.2 好友关系

| 能力 | 成熟度 | 说明 |
|---|---|---|
| 申请 / 接受 / 拒绝 / 删除好友 | 稳定 | 双向关系表，任一侧删除即解除 |
| 好友列表、好友申请列表、待处理计数 | 稳定 | |
| 拉黑 / 取消拉黑 / 黑名单查询 | 可用 | 拉黑后消息投递被拦截 |
| 好友分组（增删改查、移入移出） | 可用 | |
| 备注、星标、置顶 | 可用 | |
| 潜在好友搜索、关系查询 | 可用 | |

### 2.3 群组

| 能力 | 成熟度 | 说明 |
|---|---|---|
| 建群 / 解散 / 转让、成员增删、退群 | 稳定 | 群成员变更有缓存失效逻辑 |
| 成员角色（群主 / 管理员 / 成员） | 可用 | |
| 群设置（进群验证、全员禁言等） | 可用 | |
| 群公告增删查 | 可用 | |
| 群黑名单、单人禁言、成员设置 | 可用 | |
| 群二维码 | 可用 | |
| 我的群列表、搜索、成员检查 | 稳定 | |

### 2.4 消息

| 能力 | 成熟度 | 说明 |
|---|---|---|
| 单聊 / 群聊发送 | 稳定 | 走 Kafka 异步落库，会话内严格有序 |
| 实时推送（在线） | 稳定 | 定向路由，失败回退广播 |
| 历史消息分页（按 seq 游标） | 稳定 | 主键扫描，不随总消息量退化 |
| 会话列表 | 稳定 | 当前是 DB 拼装 + 内存排序（见第 7 节） |
| 消息已读（按会话 / 按消息） | 稳定 | 游标式，跨端同步只需同步一个数字 |
| 撤回（2 分钟内） | 可用 | 见第 6 节的行为边界 |
| 删除消息 | 可用 | |
| 转发 | 可用 | |
| 未读数 | 稳定 | `last_msg_seq - last_read_seq` 现算，不落库 |

### 2.5 文件与其它

| 能力 | 成熟度 | 说明 |
|---|---|---|
| 文件 / 图片上传（前端直传对象存储） | 稳定 | 后端只发预签名 URL，支持 COS / OSS / S3 |
| 头像上传（独立桶 + 默认头像） | 可用 | 签名有效期 1800s，文件 3600s |
| 系统消息 | 雏形 | 枚举与分支存在，实现是 TODO |
| 离线推送（APNs / FCM） | **未实现** | 离线消息落库、上线拉取，没有系统级推送通道 |

## 3. 核心用户旅程

### 3.1 注册登录

```
POST /api/users/register   → 建号（bcrypt 存密码）
POST /api/users/login      → 校验 → 签发 access/refresh token → 写 Redis(token:access:{uid}:{platform})
GET  /ws/{token}           → WS 握手时再校验一次 token 与平台，不一致 Close 4002
```

### 3.2 单聊消息

```
A 发: POST /api/chat/send
      → 返回成功时消息已进 Kafka（未落库）
      → 消费端分配 seq 并落库（通常 10~50ms）
      → 推送 {server_id, seq, ...} 给 A 和 B 的在线连接
B 收: WS 帧，按 seq 插入本地会话
B 离线: 不推送；上线后 POST /api/chat/history，body 带 conversation_id 与本地最大 seq
```

**客户端必须做的事**：以 `seq` 为唯一排序依据，不要用客户端时间；
收到推送后如果发现 `seq` 不连续，说明中间有消息没收到，需要按游标补拉。

### 3.3 群聊消息

与单聊一致，区别是推送报文对**所有成员完全一致**（`receiver_id` 是群组 ID），
方向判断用 `is_self`，会话归属用 `conversation_id`。
群成员列表在服务端缓存，消息投递时按成员集合逐个定向推送。

### 3.4 已读

```
POST /api/chat/markConversationAsRead  → 推进 user_conversation.last_read_seq
POST /api/chat/read                    → 按消息 ID 标记（内部同样推进游标）
```

未读数 = `last_msg_seq - last_read_seq`，服务端现算、不落库。

已读状态只有一个数字，所以多端同步的成本极低：任一设备推进游标，其它设备拉到新游标即可。

## 4. 对外接口

### 4.1 HTTP（api-gateway）

统一响应信封：

```json
{ "code": 0, "message": "ok", "success": true, "data": {} }
```

| 前缀 | 后端 | 说明 |
|---|---|---|
| `/api/users` | relation-service | 注册、登录、资料、搜索、配置 |
| `/api/friends` | relation-service | 好友与好友分组 |
| `/api/groups` | relation-service | 群组与群成员 |
| `/api/chat` | msg-server | 子路径即动作：`send` / `history` / `conversations` / `read`（别名 `markAsRead`）/ `markConversationAsRead` / `revoke` / `forward` 走 POST；删除消息是 `DELETE /api/chat/messages` |
| `/api/common` | 公共能力 | |
| `/api/files/upload-signature` | oss | 预签名上传（需认证） |
| `/health` | 自身 | 健康检查，返回版本号 |
| `/metrics` | 自身 | Prometheus 指标（另有 `:9090/metrics` 监听） |

默认限流：全局 5000 rps / burst 500；`/api/chat` 5000 rps；登录 5 rps / burst 3；
`/api/users` 10 rps。超限返回 429（等待秒数 >0 时才带 `Retry-After`）。
全局超时 30s，请求体上限 10MB。

> 已知缺陷：配置里的注册限流前缀是 `/api/user/register`（单数），与实际路径
> `/api/users/registerByUsername`、`/api/users/registerByPhone` 不匹配，
> 注册实际命中的是 `/api/users` 的 10 rps。

### 4.2 WebSocket（msg-gateway）

- 地址：`ws://host:8085/ws/{token}`，token 走路径（不是 header，方便浏览器端）；
- 鉴权：握手时验签 + 与 Redis 中该平台 token 比对，失败发 Close **4002**；
- 同平台重复登录：旧连接收到 Close **4001**；
- 心跳：服务端每 30s 发 Ping，未收到 Pong 的累计次数超过 `heartbeat_max_missed`（默认 2）
  即判定死链 —— 实际是连续 3 个周期（约 90s）后才断开；
- 下行帧是 **`PushPayload`**，不是裸 `Msg`：

  ```
  { conversations: [ { conversation_id, conversation_type,
                       recent_messages: [ { ...Msg 字段, is_self } ],
                       unread_count, last_active_time } ],
    total, seq_max, send_seq_max }
  ```

  `Msg` 本身没有 `conversation_id` 与 `is_self`，这两个字段由网关在拼装时补上；
- 上行帧：`ChatService.SendMsg` 的请求结构；同一会话的上行由网关按会话哈希固定 worker 串行处理，保证顺序。

### 4.3 内部 gRPC

| 服务 | 端口 | 调用方 |
|---|---|---|
| `ChatService` | 50010（`config-prod.yaml` 样例为 50002） | api-gateway、msg-gateway（上行） |
| `MsgService` | 50001 | msg-server（下行推送，打到 msg-gateway） |
| `UserService` / `FriendService` / `GroupService` | 50004 | api-gateway、msg-server（校验） |

服务发现统一走 Consul，名字只有 `relation` / `chat` / `ws` 三个。

## 5. 非功能指标（本机实测）

| 指标 | 实测 | 说明 |
|---|---|---|
| 单机长连接 | 1 万（建连成功率 100%） | 峰值 RSS 159MB，预算 512MB |
| 建连延迟 | p50 271µs / p99 2.0ms | 1 万连接、每秒爬坡 500 |
| 上行吞吐 | 9995 msg/s | 1 万连接、每连接 1 msg/s、持续 60s；异常断开 0（阶段 2 复测口径） |
| 端到端 P99 | 62~158ms（3000 msg/s / 64 会话） | 5 轮中 4 轮达标 |
| HTTP 写吞吐 | 3002 QPS，错误率 0 | 0 个 429 / 0 个 5xx |
| 优雅停机摘流 | 0s | 15s 内不复活 |
| 崩溃摘流（kill -9） | 8s | TTL 10s + 发现刷新 5s |
| RTO | ≤ 18s | 目标 30s |

这些数字是**单机、桌面环境**的结果，用于证明架构方向正确，不能直接当生产容量承诺。
生产扩容公式见 [TECHNICAL.md 第 9 节](TECHNICAL.md#9-容量模型与扩容)。

## 6. 已知行为与坑

接入前必须知道的几条，都是设计的结果而不是 bug：

| 行为 | 说明 | 客户端要怎么做 |
|---|---|---|
| 发送成功 ≠ 已落库 | 返回成功时消息在 Kafka，通常 10~50ms 后落库 | 不要立刻按「发送时间」排序，等推送里的 `seq` |
| 撤回有时序窗口 | 撤回要先查到已落库的消息；批量落库下最小窗口约 8~12ms，积压时更长。窗口内撤回会返回「消息不存在」 | 撤回失败时提示重试，不要当成消息已删除 |
| 群推送 `receiver_id` = 群组 ID | 早期版本逐成员覆写成成员 ID，属非契约行为 | 用 `conversation_id` 定位会话，`is_self` 判方向 |
| 离线不推送 | 没有 APNs/FCM 通道 | 上线后必须按 `seq` 游标补拉 |
| 跨会话无序 | 只有会话内有序 | 不要用全局时间戳做展示排序 |
| 重复推送不会发生 | 只有首次落库才推送 | 但仍建议按 `server_id` 去重（网络重连会重发） |
| 会话列表最多滞后 200ms | 水位回写限频 | 列表不要做「秒级强一致」的假设 |
| 限流是进程内 | 多实例时总量 = 配置 × 实例数 | 部署多实例时按实例数下调单实例阈值 |

## 7. 当前的限制

- **会话列表是 DB 拼装**：`user_conversation` 查出来后逐会话查 `conversation` 与最近消息，
  在内存里排序。会话几百个时可接受，上万会话需要补 Redis ZSET（设计已写入 DDL 注释，代码未实现）。
- **没有系统级推送通道**：移动端后台收不到消息。
- **没有多租户**：`tenant_id` 只出现在对象存储路径里，库表没有租户维度。
- **Redis / PG / Consul 单实例**：业务层无状态可扩，但依赖层是单点。
- **没有端到端加密**：传输层 TLS/WSS，消息在库里是明文（`content` 是 BYTEA）。
- **审计与合规能力缺失**：没有消息留存策略开关、没有操作审计日志。

## 8. 路线图

### 近期（把「注释等于实现」补齐）

1. 会话列表迁到 Redis ZSET，DB 作为冷路径。
2. 最近 N 条消息缓存，解决主从延迟与冷启动回表。
3. msg-server / msg-gateway 补 Prometheus 指标：消费延迟、死信数、在线连接数、批次耗时直方图。
4. 幂等判定并入号段脚本，减少一条消息一次 PG 查询。
5. 撤回窗口消除：撤回未命中时写入待撤回表，落库时应用（见 TECHNICAL 第 6 节的时序说明）。

### 中期（补齐产品形态）

6. 离线推送通道（APNs / FCM / 厂商通道），以及推送与在线投递的去重。
7. 群成员变更的一致性：成员变动时清理成员缓存 + 推送系统消息，避免新成员拉到旧消息。
8. 会话置顶/免打扰在服务端排序里生效（字段已存在，排序未使用）。
9. 消息搜索（内容检索需要额外索引方案，当前只有主键与时间索引）。

### 长期（规模化）

10. 多租户与按租户分片（先按 `tenant_id` 路由到不同库，再做跨租户查询治理）。
11. Redis Cluster / PG 分库分表的接入改造（号段与路由表都要改）。
12. 多地域部署与跨地域消息同步（当前所有依赖都是单集群）。
13. 端到端加密与密钥轮换。

## 9. 兼容性与升级

对外协议相对稳定，历史上唯一一次客户端可见的破坏性变更：

- 群聊推送的 `receiver_id` 语义修正为群组 ID（详见 `docs/UPGRADE_NOTES.md`）。

配置面新增都是可缺省的：`kafka.topic_partitions`、`kafka.topic_replication`、
`kafka.consumer.parallelism`、`websocket.upstream_workers`、`websocket.heartbeat_max_missed`、
`validation.fail_open`。密钥（JWT / OSS / 短信）已从配置文件移到环境变量。

Kafka 分区数只在建主题时生效，已有环境需要 `--alter --partitions` 或重建主题。
