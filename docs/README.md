# 文档索引

适用版本：**1.0.0**（版本号权威来源为 `Cargo.toml` 的 `[workspace.package] version`）

## 我该看哪一篇

| 你想做什么 | 从这里开始 |
|---|---|
| 搞清楚系统由哪些服务组成、消息怎么流转 | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 评估性能能做到多少、某段优化为什么这么改 | [TECHNICAL.md](TECHNICAL.md) |
| 找某个功能有没有实现、客户端怎么接 | [PRODUCT.md](PRODUCT.md) |
| 建库建表 | [sql/init.sql](sql/init.sql)（一个文件建完关系库 + 消息库） |
| 跑性能复测、对账基线 | [operations/BASELINE.md](operations/BASELINE.md) |
| 部署、摘流、探针参数 | [operations/CONSUL.md](operations/CONSUL.md) |
| 接日志与链路追踪 | [operations/TELEMETRY.md](operations/TELEMETRY.md) |
| 处理依赖安全告警 | [operations/SECURITY.md](operations/SECURITY.md) |
| 从旧版本升级 | [UPGRADE_NOTES.md](UPGRADE_NOTES.md) 与根目录 `CHANGELOG.md` |
| 接文件上传 | [guides/file-upload-guide.md](guides/file-upload-guide.md) |
| 接 CI/CD | [guides/ci-cd-setup.md](guides/ci-cd-setup.md) |

## 目录结构

```
docs/
├── README.md                本索引
├── ARCHITECTURE.md          架构：拓扑、链路、数据模型、一致性、取舍总表、已知落差
├── TECHNICAL.md             技术：性能模型、逐项优化、高并发/高可用实现、容量模型、调优手册
├── PRODUCT.md               产品：能力矩阵、用户旅程、接口、已知行为、限制、路线图
├── UPGRADE_NOTES.md         升级说明：客户端可见变更、配置新增、迁移注意
├── operations/              运维与验收
│   ├── BASELINE.md          性能与契约基线（不得回退的底线）
│   ├── CONSUL.md            服务注册、TTL 摘流、优雅停机
│   ├── TELEMETRY.md         日志分级与 OTLP 链路追踪
│   └── SECURITY.md          依赖安全告警台账与处置原则
├── guides/                  接入指南
│   ├── file-upload-guide.md 前端直传对象存储
│   └── ci-cd-setup.md       CI/CD 配置
└── sql/
    ├── init.sql             完整建表脚本：关系库 + 消息存储，新环境只需执行这一个
    └── archive/             历史迁移脚本，仅供追溯，不要在新环境执行
        ├── DDL-v1.sql
        ├── 20250516_update_cml_DDL.sql
        ├── 250512_update_DDL.sql
        ├── legacy-messages-init.sql   已被 init.sql 取代的旧消息单表
        └── version/                   按周推进的历史迁移
```

## 命名与维护约定

- **文档**：根目录放「描述系统本身」的三篇（架构/技术/产品）与升级说明，并保持大写命名；
  与具体工具链相关的放 `guides/`，与运行环境相关的放 `operations/`，两者都用小写连字符。
- **SQL**：新环境要执行的结构只有一个 `sql/init.sql`，新增表直接追加到对应段落，
  **不要再拆出 01/02 之类的分片脚本**；任何历史迁移、已被取代的结构一律进 `sql/archive/`，
  归档文件不改名、不改内容。
- **路径引用**：文档与代码注释里统一写仓库相对路径（`docs/sql/init.sql`），
  不要写 `./` 前缀，便于全局检索。
- **版本**：文档不单独标版本，统一以本文件顶部的版本号为准；破坏性变更先写
  `UPGRADE_NOTES.md`，再追加根目录 `CHANGELOG.md`。

## 文档里刻意保留的「已知落差」

架构注释、DDL 注释和文档之间存在若干「设计意图已写、代码未实现」的地方
（会话列表 Redis ZSET、最近消息缓存、分布式锁等）。它们被集中记录在
[ARCHITECTURE.md 第 9 节](ARCHITECTURE.md#9-已知落差注释与实现不一致的地方)，
而不是从文档里删掉——避免下一个人按注释做设计。
