# 配置说明

配置只经 `runtime::config::AppConfig::from_file` 读取，再以构造参数注入各模块；
不存在进程级全局单例，也不支持配置热重载（修改配置需重启进程）。

## 文件

| 文件 | 用途 |
|---|---|
| `config.yaml` | 本地开发默认配置 |
| `config-prod.yaml` | 生产环境配置样例 |
| `filebeat/`、`logstash/` | 日志采集链路（`docker-compose.telemetry.yml` 挂载） |
| `legacy/` | 配置中心时代遗留文件归档，不参与运行 |

## 用法

```rust
let config = std::sync::Arc::new(runtime::config::AppConfig::from_file(None)?);
// 把 config 传给各服务的构造函数，例如 ServiceProxy::new(config.clone())
```

`from_file(None)` 会优先读取 `./config/config.yaml`；也可显式传入路径。

## 敏感信息

密钥不在配置文件中落盘，由以下环境变量覆盖（存在且非空时优先级更高）：

| 环境变量 | 覆盖字段 |
|---|---|
| `OSS_ACCESS_KEY` | `oss.access_key` |
| `OSS_SECRET_KEY` | `oss.secret_key` |
| `SMS_SECRET_ID` | `sms.tencent.secret_id` |
| `SMS_SECRET_KEY` | `sms.tencent.secret_key` |
| `JWT_SECRET` | `gateway.auth.jwt.secret` |

## 关键配置项

- `websocket.upstream_workers`：上行并行 worker 数（默认 8，组内保序）
- `websocket.heartbeat_max_missed`：连续未收到 Pong 的心跳周期阈值（默认 2）
- `kafka.topic_partitions` / `kafka.topic_replication`：主题分区/副本（默认 16/3，仅在创建主题时生效）
- `kafka.consumer.parallelism`：消费端并行度
- `validation.fail_open`：权限校验链路故障时是否放行（默认 false）
