# RustIM

[![version](https://img.shields.io/badge/version-1.0.0-blue.svg)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

用 Rust 实现的云原生即时通讯后端。四个可部署服务、五个库 crate，
以「读扩散存储 + Kafka 异步落库 + WebSocket 长连接网关」为主干，
所有性能结论都有本机实测数据支撑（见 [docs/operations/BASELINE.md](docs/operations/BASELINE.md)）。

当前版本 **1.0.0**：版本号唯一权威来源是 `Cargo.toml` 的 `[workspace.package] version`，
`GET /health` 返回的 `version` 即该值；变更记录见 [CHANGELOG.md](CHANGELOG.md)。

**本机实测（10 核 16GB macOS，中间件跑在 Docker）**

| 维度 | 结果 |
|---|---|
| 单机长连接 | 1 万连接，建连成功率 100%，峰值 RSS **159MB** |
| 建连延迟 | p50 271µs / p99 2.0ms |
| 上行吞吐 | **9995 msg/s** 持续 60s（阶段 2 复测：1 万连接异常断开 0） |
| 端到端推送 P99 | **62~158ms**（3000 msg/s / 64 会话，5 轮中 4 轮达标） |
| HTTP 写吞吐 | **3002 QPS**，错误率 0，无限流误伤 |
| 故障恢复 | 优雅停机摘流 **0s**，kill -9 摘流 8s，RTO ≤ 18s |

优化前同口径端到端 P99 是 7.5s 且积压线性增长。怎么做到的见
[docs/TECHNICAL.md](docs/TECHNICAL.md)。

## 架构

```
        HTTP                                    WebSocket
          │                                         │
   ┌──────▼───────┐                        ┌────────▼────────┐
   │ api-gateway  │  JWT / 限流 / 熔断 / 路由 │   msg-gateway   │ 连接表 / 定向投递
   └──┬────────┬──┘                        └───┬─────────┬───┘
      │ gRPC   │ gRPC                          │ 上行     │ 下行
┌─────▼──────┐ └──────────►┌──────────────────┐◄┘         │
│  relation  │             │    msg-server    │───────────┘
│ 用户/好友/群 │             │ 生产 + 消费 + 推送 │
└─────┬──────┘             └───┬──────────┬───┘
      │                        │          │
     PG                     Kafka       Redis
                                 ▲
                        Consul（注册 / 发现）
```

| 服务 | 端口 | 职责 |
|---|---|---|
| `api-gateway` | HTTP 8000（容器对外 8080） | 统一入口、认证、限流、熔断、文件签名 |
| `msg-gateway` | WS 8085 / gRPC 50001 | 长连接接入、连接注册表、消息下行 |
| `msg-server` | gRPC 50010 + Kafka | 消息生产、批量落库、推送路由（`config-prod.yaml` 样例写的是 50002） |
| `relation-service` | gRPC 50004 | 用户 / 好友 / 群组（模块化单体） |

`contract` / `runtime` / `cache` / `msg-storage` / `oss` 是库 crate，没有独立进程。

## 快速开始

### 方式一：本机跑服务，中间件用容器（开发推荐）

```bash
# 1. 依赖：Rust 1.85+、Docker
docker compose -f docker-compose.dev.yml up -d     # PG / Kafka / ZK / Consul
# Redis 复用本机实例或自行启动

# 2. 运行环境变量（.env 里只有 DATABASE_URL 等基础项；
#    JWT_SECRET / OSS / 短信密钥在启动前导出，不要写回配置文件）
set -a; source .env; set +a
export JWT_SECRET=$(openssl rand -hex 32)

# 3. 建表（关系库 + 消息库一次建完）
psql "$DATABASE_URL" -f docs/sql/init.sql

# 4. 构建 + 启动四个服务
./scripts/build.sh release
./scripts/start-services.sh start     # status / stop / restart

# 5. 验证
curl -s localhost:8000/health
./tests/contract/api_contract.sh      # 9 项契约门禁，必须全绿
```

### 方式二：全容器

```bash
docker compose -f docker-compose.prod.yml up -d
curl -s localhost:8080/health
```

`docker-compose.prod.yml` 会带起 Consul、Kafka 3 broker（`min.insync.replicas=2`）、
PG16、nginx 与四个业务服务；密钥通过环境变量注入，不要写回配置文件。

### Kubernetes

```bash
kubectl apply -f k8s/00-namespace.yaml
kubectl apply -f k8s/01-secrets.yaml
kubectl apply -f k8s/            # consul / postgres / redis / kafka / 四个业务服务
kubectl get pods -n rustim
```

清单含 Deployment / Service / HPA / PDB，探针与摘流预算见 [k8s/README.md](k8s/README.md)。

## 目录结构

```
chrisIM_rust/
├── api-gateway/        HTTP 入口：认证、限流、熔断、路由转发、文件签名
├── msg-gateway/        WebSocket 接入 + 内部 gRPC 推送（含 ws_bench / e2e_bench）
├── msg-server/         消息生产、Kafka 消费、批量落库编排、推送
├── relation-service/   用户 / 好友 / 群组（模块化单体）
├── msg-storage/        读扩散存储实现：批量写、幂等、号段、水位
├── cache/              Redis 领域封装（号段/会话/关系/在线/未读）
├── oss/                对象存储抽象（S3 / COS / OSS，预签名直传）
├── contract/           proto、领域类型、统一错误、会话键推导
├── runtime/            配置、日志、服务注册发现、gRPC 客户端、优雅停机、分段计时
├── config/             多环境配置样例
├── tests/              契约测试 / 故障注入 / 性能基准
├── scripts/            构建与进程管理脚本
├── k8s/                K8s 清单
├── CHANGELOG.md        版本变更记录
└── docs/               文档与 SQL（索引见 docs/README.md）
```

## 文档索引

完整导航（含「我想做什么 → 看哪篇」）见 [docs/README.md](docs/README.md)。

| 文档 | 内容 |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 系统拓扑、消息链路、数据模型、一致性语义、关键取舍总表、已知落差 |
| [docs/TECHNICAL.md](docs/TECHNICAL.md) | 性能模型与逐项优化（问题→做法→效果→代价）、高并发/高可用实现、容量模型、调优手册 |
| [docs/PRODUCT.md](docs/PRODUCT.md) | 能力矩阵与成熟度、用户旅程、对外接口、已知行为坑、限制与路线图 |
| [docs/operations/BASELINE.md](docs/operations/BASELINE.md) | 性能与契约基线，每次重构不得回退的底线 |
| [docs/operations/CONSUL.md](docs/operations/CONSUL.md) | 服务注册、TTL 摘流、优雅停机 |
| [docs/operations/TELEMETRY.md](docs/operations/TELEMETRY.md) | 日志分级与 OTLP 链路追踪 |
| [docs/sql/init.sql](docs/sql/init.sql) | 完整建表脚本（关系库 + 消息存储，一次执行） |
| [docs/UPGRADE_NOTES.md](docs/UPGRADE_NOTES.md) · [CHANGELOG.md](CHANGELOG.md) | 升级注意与版本变更记录 |
| [docs/guides/file-upload-guide.md](docs/guides/file-upload-guide.md) | 文件上传（前端直传） |
| [docs/guides/ci-cd-setup.md](docs/guides/ci-cd-setup.md) | CI/CD |

## 性能设计要点

一句话版本：**吞吐由 SQL 语句数决定，延迟由攒批窗口和热点行更新决定。**

- **一条消息只写一次**（读扩散）：万人群发一条消息 2 行写入，写扩散是 10001 行。
- **批量落库**：整批一条判重 + 一条多行 `INSERT ... ON CONFLICT DO NOTHING RETURNING`，
  语句数从「每条 2~3 条」降到「每批 2 条」。这是 P99 从 7.5s 降到百毫秒的主因。
- **Redis 号段**：热群不再有 `FOR UPDATE` 行锁争用；步长放大到 1000 让跨界回写从 30 次/秒降到 3 次/秒。
- **水位按批 + 200ms 限频**：水位段从 100ms 降到 18µs，退出消费热路径。
- **攒批窗口自适应**：满载 12ms、空闲 10ms、每 8ms 无新消息即收手，P50 从 117ms 降到 23ms。
- **WS 缓冲 8KB + 单写任务 + 有界队列**：1 万连接 RSS 从 1313MB 降到 159MB。
- **上行按会话哈希并行、群推单次序列化**：异会话并行、同会话保序，万人群内存分配 O(n)→O(1)。

代价与失败语义（批量失败会不会连坐、撤回窗口为什么存在）在
[TECHNICAL.md 第 6 节](docs/TECHNICAL.md#6-一致性细节批量写失败会连坐吗) 讲清楚。

## 配置

主配置 `config/config.yaml`，通过 `CONFIG_PATH` 指定其它文件。分段：
`gateway`（路由表）/ `log` / `telemetry` / `database` / `message_storage` /
`server` / `validation` / `service_center` / `websocket` / `rpc` / `redis` / `kafka` / `oss` / `sms`。

密钥走环境变量：`JWT_SECRET`、`OSS_ACCESS_KEY`、`OSS_SECRET_KEY`、
`SMS_SECRET_ID`、`SMS_SECRET_KEY`，以及 `ADVERTISE_HOST`、`CONSUL_HOST`、`CONSUL_PORT`。
详见 [config/README.md](config/README.md)。

诊断开关：`IM_STAGE_PROFILE=1` 打开分段耗时采样（默认关闭，关闭时零开销）。

## 测试与门禁

```bash
cargo test --workspace                      # 单测（DB 相关用例自动 ignored）
./tests/contract/api_contract.sh            # 对外 HTTP 契约 9 项
./tests/faults/rto-consul.sh ws msg-gateway kill9 20
./tests/faults/dependency-down.sh redis 20   # 依赖故障注入
JWT_SECRET=<same> ./tests/perf/e2e.sh 3000 64 15 10   # 端到端基准
cargo audit                                  # 依赖安全审计
```

CI（`.github/workflows/main.yml`）：Redis + Kafka 服务容器 → clippy（两个消息服务 `-D warnings`）
→ 单测 → release 分支跑 `cargo audit` → 镜像构建 → Trivy 扫描。

## 后续优化与升级方向

### 近期（补齐「注释等于实现」）

1. **幂等判定并入号段脚本**：用同一次 Redis 往返里的 `SET NX` 判重，去掉每条消息一次 PG 预查询。
2. **会话列表上 Redis ZSET**：当前是 DB 拼装 + 内存排序，会话上万后退化；DDL 注释已写但代码未实现。
3. **最近 N 条消息缓存**：`hot_cache_messages` 目前只有配置项，没有实现，主从延迟下「刚发就拉不到」。
4. **可观测性补齐**：msg-server / msg-gateway 没有 Prometheus 指标，消费延迟、死信数、在线连接数只能从日志推算。
5. **批量冲突兜底**：`ON CONFLICT` 扩到覆盖主键，加批量前预校验，消除「一条脏数据拖整批重试」。

### 中期（补齐产品形态）

6. **离线推送**：APNs / FCM / 厂商通道，以及与在线投递的去重（当前移动端后台收不到消息）。
7. **撤回窗口消除**：撤回未命中时写待撤回表，落库时应用，让「发完立刻撤回」不再失败。
8. **群成员变更一致性**：成员变动清理缓存 + 系统消息，避免新成员拉到旧消息。
9. **会话置顶/免打扰参与排序**（字段已存在，排序未使用）、消息内容检索。

### 长期（规模化）

10. **多租户**：`tenant_id` 目前只出现在对象存储路径，库表还没有租户维度。
11. **Redis Cluster / PG 分片**：号段与在线路由都要跟着改。
12. **多地域部署与跨地域同步**：当前所有依赖都是单集群。
13. **端到端加密与密钥轮换**：现在只有传输层加密。

## 贡献

1. Fork 并新建分支
2. `cargo fmt && cargo clippy --workspace`
3. 补测试：行为变更必须有单测，对外契约变更必须更新 `tests/contract/api_contract.sh`
4. 改性能路径请附上 `docs/operations/BASELINE.md` 口径的前后对比

## 许可证

MIT，见 [LICENSE](LICENSE)。
