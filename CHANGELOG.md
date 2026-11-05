# 更新日志

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)：主版本号表示对客户端或部署方式的破坏性变更，
次版本号表示向后兼容的能力新增，修订号表示修复与内部优化。

版本号的唯一权威来源是 `Cargo.toml` 的 `[workspace.package] version`，各 crate 通过
`version.workspace = true` 继承；`GET /health` 返回的 `version` 字段即该值。

## [未发布]

### api-gateway 重构（错误契约 / 故障隔离 / 可观测性）

**破坏性变更：对外 HTTP 状态码**

此前所有失败都是 500，且「令牌失效」会返回 `HTTP 200 + {"code":401,"success":true}`。
现在统一为 `{code,message,success:false}` 且 **HTTP 状态码 == `code`**：

| 场景 | 之前 | 现在 |
|---|---|---|
| 参数缺失 / 格式错误 / 畸形 JSON | 500 | **400** |
| 令牌无效 / 凭据错误 | 200（`success:true`）或 500 | **401** |
| 越权 / 企业号不匹配 | 500 | **403** |
| 未知接口 / 未实现方法 | 500 | **404** |
| 下游不可用 / 熔断 | 503 | **503** |
| 下游报错、超时 | 500 | **502 / 504** |

客户端如果只读 HTTP 状态码，请按上表适配；body 结构保持 `{code,message,success}` 不变。

- **统一错误类型**：新增 `api-gateway/src/error.rs`，`From<tonic::Status>` 把 gRPC 码映射成
  HTTP 语义；内部细节（Status 原文、连接串）只进日志，不再回给客户端。
- **响应信封修正**：`success_response` 去掉形同虚设的状态码参数，成功恒 200；
  限流 429 也改用同一信封并带 `Retry-After`（亚秒等待向上取整到 1）。
- **登录**：凭据错误统一 401，文案不再区分「用户不存在/密码错误」（消除账号枚举）；
  下游不可用时如实 503。
- **请求解析**：畸形 JSON 直接 400（此前静默退化成 query 参数）；query 做百分号解码；
  body 上限与 `RequestBodyLimitLayer` 统一为 10MB。
- **故障隔离**：gRPC 客户端按需**单点重建** + 指数退避（1s→30s）。
  此前后端宕机时每个请求重建全部 4 条连接（含 4 次 Consul 查询），且每次重建泄漏一个
  永久轮询任务；`DynamicServiceDiscovery::run` 现在在接收端关闭时退出。
- **中间件顺序**：`CORS → 体积 → 超时 → 访问日志 → 指标 → 限流 → 熔断 → 认证`。
  此前日志/指标在限流之内，429/413/408/熔断响应都不进指标也不进访问日志。
  移除重复的 `TraceLayer`。
- **指标**：`path` 标签归一化（`/api/users/{id}/*`），`service` 用真实下游名
  （`relation`/`chat`/`gateway`），消除按用户 ID 的时间序列爆炸；
  删掉 `with_http_listener(9090)` 这段从未生效的配置；recorder 装不上时降级而不是 panic。
- **安全**：JWT 原文不再写 INFO 日志（改记长度 + 指纹）；`auth.ip_blacklist` 从 TODO 变为生效；
  路径白名单/限流前缀改为按路径段边界匹配（`/api/user/loginXXX` 不再命中白名单）。
- **IP 口径统一**：新增 `api_utils::net`，对端地址为权威，只有 `trusted_proxies` 命中时
  才采信 `X-Forwarded-For`；日志、限流、黑名单共用。
- **配置**：新增 `gateway.cors`（来源/凭证）与 `gateway.auth.trusted_proxies`；
  CORS 来源不再硬编码开发端口。
- **启动/停机**：serve 失败返回非 0 退出码（此前打一条日志后 `Ok(())`，编排层无法感知）；
  错误路径不再挂死等待信号；`server.host` 真正参与绑定。
- **代理层**：新增 `ChatServiceGrpcClient`，user / friend / group / chat 四个下游统一经
  `proxy/grpc_clients` 调用，业务处理器不再直接持有 tonic 客户端；
  chat 的转发失败也走 `GatewayError` 映射，不再固定 502。
- **清理**：删除死代码 `api_utils/ip_region`（含 `ip2region` 依赖）、3 个未使用 DTO、
  2 个重复的转换函数与若干未使用方法；`CommonServiceHandler` 去掉只收不用的 user 客户端。


按 `docs/TECHNICAL.md` 9.2 的 ROI 表，处理成本/风险为「极低」「低」的条目。

### 新增

- **msg-server / msg-gateway 导出 Prometheus 指标**：新增 `runtime::metrics_server`，
  两个消息服务各起独立监听（msg-server `:9101`、msg-gateway `:9102`，
  由配置 `metrics.enabled` / `metrics.port` 控制，`port: 0` 表示用服务默认端口）。
  - msg-server：`msg_consume_batch_total{result}`、`msg_consume_batch_size`、
    `msg_consume_batch_duration_seconds`、`msg_inserted_total`、`msg_duplicate_total`、
    `msg_precheck_rejected_total`、`msg_push_total{result}`、`msg_dead_letter_total`；
  - msg-gateway：`ws_connections`、`ws_upstream_total`、`ws_downstream_total`、
    `ws_outbound_dropped_total`、`ws_dead_conn_total`。
  - 用 `PrometheusBuilder::build()` + 手工 spawn exporter：`install_recorder()` 只装 recorder
    不 spawn 监听，用它的话 `/metrics` 端口永远连不上（新增回归用例覆盖）。
- **熔断器半开态探测限流**：`half_open_max_probes`（默认 3）限制同时放行的探测请求，
  `half_open_success_threshold`（默认 2）要求连续成功达到次数才关闭熔断器；
  两个字段都有 serde 默认值，旧配置文件无需改动。
- **批量落库预校验与全冲突兜底**：`msg_meta` 写入改为无 target 的 `ON CONFLICT DO NOTHING`，
  同时覆盖 `(conversation_id, server_id)` 与主键 `(conversation_id, seq)`；
  `server_id`/`conversation_id` 为空或 `content` 超过 `INT` 上限的消息不进批量，
  直接转逐条（有重试与死信），并计入 `msg_precheck_rejected_total`。
- **会话列表索引**：新增 `cache::ConversationIndexCache`，每个用户一个 ZSET `conv_list:{user_id}`
  （score 为最近活跃时间，`ZADD ... GT` 只增不减，30 天过期）。消费端首次落库后按参与者批量推进索引；
  会话列表改为「Redis 给顺序、SQL 批量补游标」，新增 `ConversationRepo::get_many` 与
  `UserConversationRepo::list_by_ids`，单次请求的查询次数从 `1 + 2×会话数` 降到常数。
  索引缺失时回落到数据库并后台回填。
- **未读计数整批写**：会话列表不再按会话逐个写 `unread_count`，改为一整批 `HSET/HDEL` pipeline；
  同时删掉 `HLEN + DEL` 的空 Hash 清理（Redis 删空 field 后会自动回收键，这两次调用从不生效）。
- **位点提交节流**：`commit_offsets` 把各分区最大位点攒起来，攒够 8 批或超过 200ms 才提交一次
  （原先每批提交一次）。未能落库的分区会先丢弃已攒位点再 `seek`，提交失败保留位点重试，
  语义仍是 at-least-once。

### 修复

- **注册限流前缀不生效**：`config.yaml` / `config-prod.yaml` 的规则前缀由 `/api/user/register`
  改为 `/api/users/register`，注册终于拿到 2 rps 的独立保护（此前混在 `/api/users` 的 10 rps 里）。
- **心跳死链注释口径**：统一为「未收到 Pong 的累计次数 > `heartbeat_max_missed` 才断开
  （默认第 3 个周期，约 90 秒）」，配置与两个压测工具的行内注释不再写「约 60 秒」。
- **会话列表顺序**：`GetConversations` 改为「最近活跃在前」。此前按 `last_active_time` 升序输出，
  且 SQL 没有 `ORDER BY`，`LIMIT` 取到哪 50 个会话不确定。
- **熔断器分桶与作用域**（`api-gateway`）：
  - 分桶键改用真实服务名 `relation` / `chat`。此前用 `user-service` / `friend-service` /
    `group-service`，把同一个下游拆成 3 个熔断器，失败要分别攒够阈值，实际保护被稀释成 1/3；
  - 只对真正转发的路径建熔断（`/api/users`、`/api/friends`、`/api/groups`、`/api/chat`）。
    此前 `/health`、`/metrics`、登录/刷新/登出与 404 全落进 `unknown-service`，
    网关自身一个 5xx 就能把登录接口一起熔断成 503（自伤）；
  - 熔断器表改由 `CircuitBreakerLayer` 持有并分发给各路由。`Router::layer` 会对每条路由
    各调一次 `layer()`，表建在中间件里等于按路由条数放大阈值（`/api/users` 熔断后
    `/api/groups` 照样放行）；
  - 删除 `X-Target-Service` 请求头入口：客户端可任意注入，会在 breaker 表里无限新建记录；
  - `gateway.circuit_breaker.enabled` 终于生效，配 `false` 时整体透传；
  - 熔断的 503 响应改用统一信封 `{code,message,success}`，不再返回 `{"error":503,...}`。

## [1.0.0] - 2026-11-05

首个版本。阶段 0（验收网）、阶段 1（可用性）、阶段 2（性能）全部完成并留下基线数据。

### 新增

- **跨会话批量落库**：`save_batch_multi` 把整批消息压成「一条判重 + 一条多行写入」，
  语句数与消息数解耦。3000 msg/s / 64 会话下 P99 从 7.5s 降到 62~158ms。
- **分段耗时采样**：`runtime::stage_profile`，`IM_STAGE_PROFILE=1` 开启，
  七段（等待/幂等/号段/写入/推送/端到端/生产）各自 p50/p90/p99，默认关闭且零开销。
- **端到端基准**：`e2e_bench` 的 `e2e` / `qps` 两种模式，`tests/perf/e2e.sh` 固化复测口径。
- **文档体系**：`docs/ARCHITECTURE.md`、`docs/TECHNICAL.md`、`docs/PRODUCT.md`，
  以及按 `operations/` / `guides/` / `sql/` 分层的其余文档。

### 变更

- 版本号统一到 workspace：9 个 crate 由 `0.1.0` 升为 `1.0.0`，`/health` 的 version 随之变化。
- `docs/` 重新组织：可执行的建表语句合并为单文件 `docs/sql/init.sql`（关系库 + 消息存储，
  新环境一次执行），运维文档移入 `docs/operations/`，历史迁移 SQL 归档到 `docs/sql/archive/`。
- `Cargo.lock` 纳入版本控制（二进制项目应锁定依赖），Dependabot 与 CI 复现依赖它。

### 修复

- **tokio 1.53.1 → 1.54.1**：RUSTSEC-2026-0386（`AsyncFd` 安全构造函数违反 I/O safety，
  fd 编号复用后可致 UAF）。`cargo audit` 只把它算作 warning，此前未拦住。
- **移除 `ttf-parser`**（RUSTSEC-2026-0192，unmaintained）：验证码改为内嵌 5×7 点阵字模
  自绘，去掉 `imageproc` / `ab_glyph` / 350KB 字体文件，`runtime` 只保留 `image` 做 PNG 编码。
- **移除 `rustls-pemfile`**（RUSTSEC-2025-0134，unmaintained）：`api-gateway` 不再启用
  `axum-server` 的 `tls-rustls` feature —— 网关始终明文 HTTP，TLS 由外层 nginx/LB 终止。
- `rsa` 的 Marvin 告警（RUSTSEC-2023-0071）确认不可达：来自 `sqlx` 声明的可选 mysql 驱动，
  `cargo tree -i rsa` 无输出，默认构建不编译该路径，保留豁免并记录在
  `docs/operations/SECURITY.md`。
- 清理 `cargo audit` 中已失效的 rustls-webpki 0.101 豁免（依赖链已随 aws-smithy-http-client
  1.5.2 迁到 rustls 0.23）。
- 删除误入仓库的服务器凭据文档。**该文件仍存在于 git 历史中，需单独处理。**

### 已知问题

- ~~注册限流配置前缀 `/api/user/register` 与实际路径 `/api/users/register*` 不匹配，
  注册实际走 `/api/users` 的 10 rps。~~（未发布版本已修复）
- 会话列表的 Redis ZSET、最近消息缓存只有配置/注释，没有实现。
- ~~msg-server / msg-gateway 没有 Prometheus 指标导出。~~（未发布版本已补上）
- `config-prod.yaml` 的 chat 端口是 50002，与其余配置（50010）不一致。

完整清单见 `docs/ARCHITECTURE.md` 第 9 节与 `docs/TECHNICAL.md` 第 9 节。
