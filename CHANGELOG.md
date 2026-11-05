# 更新日志

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)：主版本号表示对客户端或部署方式的破坏性变更，
次版本号表示向后兼容的能力新增，修订号表示修复与内部优化。

版本号的唯一权威来源是 `Cargo.toml` 的 `[workspace.package] version`，各 crate 通过
`version.workspace = true` 继承；`GET /health` 返回的 `version` 字段即该值。

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

- 注册限流配置前缀 `/api/user/register` 与实际路径 `/api/users/register*` 不匹配，
  注册实际走 `/api/users` 的 10 rps。
- 会话列表的 Redis ZSET、最近消息缓存只有配置/注释，没有实现。
- msg-server / msg-gateway 没有 Prometheus 指标导出。
- `config-prod.yaml` 的 chat 端口是 50002，与其余配置（50010）不一致。

完整清单见 `docs/ARCHITECTURE.md` 第 9 节与 `docs/TECHNICAL.md` 第 9 节。
