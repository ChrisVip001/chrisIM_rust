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

## 容器环境

配置里的 `127.0.0.1` 是本地监听地址，注册相关地址由环境变量覆盖：

| 环境变量 | 覆盖字段 | 说明 |
|---|---|---|
| `ADVERTISE_HOST` | `advertise_host` | 注册到 Consul 的地址；k8s 取 `status.podIP`，compose 由 `scripts/docker-entrypoint.sh` 按容器 IP 自动注入；缺省时沿用监听 host |
| `CONSUL_HOST` | `service_center.host` | 注册中心地址；缺省的 127.0.0.1 在 Pod 内指向自己 |
| `CONSUL_PORT` | `service_center.port` | 注册中心端口 |

不注入 `ADVERTISE_HOST` 时，多副本会全部注册成 127.0.0.1，服务发现把流量导向调用方自己。

## 关键配置项

- `websocket.upstream_workers`：上行并行 worker 数（默认 8，组内保序）
- `websocket.heartbeat_max_missed`：连续未收到 Pong 的心跳周期阈值（默认 2）
- `kafka.topic_partitions` / `kafka.topic_replication`：主题分区/副本（默认 16/3，仅在创建主题时生效）
- `kafka.consumer.parallelism`：消费端并行度
- `validation.fail_open`：权限校验链路故障时是否放行（默认 false）
- `database.postgres.pool_size`：每服务进程的 PG 连接池上限（默认 32，代码限幅 [4,64]）；默认池只有 10，会卡住消费端按会话并行
- `gateway.rate_limit`：当前 global / `/api/chat` 均为 5000rps，按单机 3000 QPS 容量目标设定；governor 是进程内限流，多实例部署需按实际容量重设
- `service_center.timeout`：注册中心 HTTP 请求超时（秒），代码限幅 [1,10]，防止误填端口号
- `service_center.service_refresh_interval`：发现端刷新间隔（秒），代码限幅 [1,30]，默认 5
- `*.health_check.interval`：TTL 判定窗口（默认 10s），进程内上报周期自动取窗口的 1/3；`deregister_after` 会被提升到 60s（Consul 下限），只用于清理僵尸注册，不参与摘流
