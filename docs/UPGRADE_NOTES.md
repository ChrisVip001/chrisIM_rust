# 高并发/高可用改造升级说明与压测指南

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](README.md) · 变更记录：`CHANGELOG.md`

本轮改造目标：高并发、高性能、高可用，同时保持代码结构简洁优雅（高内聚低耦合）。
不新增服务进程、不引入新中间件。

## 一、客户端可见的协议变更

群聊推送的 `receiver_id` 语义修正为与 proto 注释一致（`群聊时是群组ID`）：

- **旧行为**：msg-gateway 在群推送时逐成员把 `msg.receiver_id` 覆写为成员自己的 ID。
- **新行为**：推送报文对全部成员完全一致（`receiver_id` 保持 msg-server 下发的值），
  群内定位会话请使用 `conversation_id`（= `group_id`），方向判断使用 `is_self` 标记。
- 若客户端此前依赖"群推送报文里 receiver_id 是我自己"这一非契约行为，需要同步修正。

其余对外协议（WS 帧格式、HTTP API、内部 proto）不变。

## 二、服务端架构变化摘要

| 改造点 | 位置 | 效果 |
|---|---|---|
| 上行按会话哈希 N worker 并行 | `msg-gateway/src/manager.rs` | 消除单消费者串行瓶颈；同会话保序，异会话并行；`websocket.upstream_workers` 可配 |
| WS 写路径去锁 | `msg-gateway/src/client.rs`、`ws_server.rs` | `RwLock<SplitSink>` → per-connection 有界队列 + 唯一写任务，天然背压 |
| 群推送单次序列化 | `msg-gateway/src/manager.rs` | 报文序列化 1 次，`Utf8Bytes`(Bytes) 引用计数共享，万人群内存分配 O(n)→O(1) |
| Kafka 多分区多副本 | `msg-server/src/kafka.rs`、`config.yaml` | `topic_partitions`(默认16)/`topic_replication`(默认3) 配置化；副本数自动收敛到 broker 数 |
| 消费端按分区扩 worker | `msg-server/src/consumer.rs` | 同组多消费者，分区自动均衡；`kafka.consumer.parallelism`(默认4) |
| 推送定向路由 | `msg-server/src/pusher/service.rs` | Redis 路由表（`user_gateway_route:{user}`），定向投递失败/缺失回退广播 |
| 权限校验缓存化+并行 | `msg-server/src/productor/validation.rs` | 正常路径零 RPC；`validation.fail_open` 可配故障降级 |
| 统一优雅停机 | `runtime/src/shutdown.rs` | 四服务 SIGTERM/SIGINT → 摘 Consul 流量 → 排空 → 退出 |
| 心跳死链检测 | `msg-gateway/src/ws_server.rs` | 连续 N 个周期未收 Pong 断开（`heartbeat_max_missed`，默认 2） |
| 雪花 ID 去阻塞 | `runtime/src/snowflake.rs` | 临界区只做算术，等待在锁外；并发唯一性有测试覆盖 |
| api-gateway 客户端池自愈 | `api-gateway/src/proxy/service_proxy.rs` | 启动失败不再永久 503，请求路径按需重建连接池 |

## 三、代码结构收敛

- `Cache` 47 方法上帝接口 → 5 个领域 trait（`cache/src/traits.rs`：Seq/Session/Relation/Presence/Unread），`Cache` 作为组合入口，`Arc<dyn Cache>` 调用方零改动
- `redis/mod.rs`(43KB) → 5 个领域实现文件 + 共享基础设施
- `productor.rs`(54KB) → 校验拆至 `productor/validation.rs`
- `logging.rs`(38KB) → `logging/{mod,init,telemetry}.rs`
- `register_service` 六分支查表化；api-gateway 死代码删除（`config/`、`tracing/`、`api_doc/`、`auth/jwt.rs`，含重复限流配置与 19KB 未接线的 api_docs）
- MongoDB 注释漂移修正；云密钥/JWT 密钥外移环境变量（`OSS_ACCESS_KEY`/`OSS_SECRET_KEY`/`SMS_SECRET_ID`/`SMS_SECRET_KEY`/`JWT_SECRET`）
- 编排：`docker-compose.prod.yml` 删除已合并的 user/friend/group 容器、新增 Consul、Kafka 3 broker；新建 `k8s/` 全套 manifests（StatefulSet/Deployment/HPA/PDB/Secret）

## 四、压测验证（ws_bench）

改造前基线与改造后对比请用同一命令采集：

```bash
# 1. 起依赖（Redis 参与 token 校验；本压测不依赖 Kafka/PG）
docker compose up -d redis consul

# 2. 启动网关后运行压测：1 万连接，每秒爬坡 500，持续 60s，每连接每秒 1 条上行
cargo run --release -p msg-gateway --bin ws_bench -- \
    --config ./config/config.yaml \
    --connections 10000 --ramp 500 --duration 60 --rate 1
```

关注指标与预期方向：

| 指标 | 预期变化 | 依据 |
|---|---|---|
| 建连成功率/延迟分位 | 持平或更好 | 连接生命周期路径未变慢，写路径排队更短 |
| 上行吞吐（msg/s） | 显著提升 | 单消费者 → N worker 并行；瓶颈转移至 Kafka（分区≥16） |
| 推送 P99 | 显著下降（万人群） | 单次序列化 + 零拷贝共享 + 定向路由 |
| 异常断开数 | 应为 0 | 死链检测只清理假在线，不影响活跃连接 |

> 表中数值列请在你的环境实测后填写；建议改造前后各跑三轮取中位数。

## 五、部署迁移注意

- **Kafka 分区变更需重建主题**：`kafka.topic_partitions` 只在创建主题时生效；
  既有环境执行 `kafka-topics --alter --partitions 16` 或删主题重建（dev 自动，prod 择窗口）。
- **配置新增段**（均可缺省）：`kafka.topic_partitions`、`kafka.topic_replication`、
  `kafka.consumer.parallelism`、`websocket.upstream_workers`、`websocket.heartbeat_max_missed`、`validation.fail_open`。
- **密钥迁移**：将 config.yaml 中的 oss/sms/jwt 密钥移入环境变量（见上表），
  config.yaml 已置空并注释。
- **compose**：`docker compose -f docker-compose.prod.yml up -d`；
  首次部署 Consul 需先于业务服务就绪（编排已用 healthcheck 排序）。
