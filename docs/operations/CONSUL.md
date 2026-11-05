# Consul 服务注册与发现

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](../README.md)

本文描述当前实现（`runtime/src/service_register_center/`、`runtime/src/service_discovery/`），
不是方案设想。历史版本曾用 `common/src/service_registry.rs` 的 HTTP 健康检查端口方案，
已随拆 crate 一并移除。

## 1. 注册了什么

对外只暴露三个服务名，全部由 `runtime` 统一注册：

| 服务名 | 注册方 | 地址来源 | 用途 |
|---|---|---|---|
| `relation` | `relation-service` | `rpc.user.host:port`（历史字段名，端口 50004） | 用户 / 好友 / 群组 |
| `chat` | `msg-server` | `rpc.chat.host:port`（50010） | 消息上行与查询 |
| `ws` | `msg-gateway` | `rpc.ws.host:port`（50001） | 下行推送 |

`msg-gateway` 的 WebSocket 端口（8085）不参与 gRPC 服务发现，只用于客户端连接；
跨节点推送走 `ws` 服务名。

注册地址取 `ADVERTISE_HOST`（容器内由入口脚本注入容器 IP，k8s 用 `status.podIP`），
不是 `127.0.0.1` —— 否则健康检查与发现都会指向调用方自己。

## 2. 健康检查：TTL 自报

注册时带 `check` 为 TTL 类型，由进程自己周期上报：

```yaml
health_check:
  health_type: "ttl"
  interval: 10          # TTL 判定窗口
  timeout: 2
  deregister_after: 60  # 只用于清理僵尸注册
```

- **上报周期 = TTL / 3（3s）**。早期窗口与上报都是 15s，一次抖动就误判 critical（踩点续命）。
- `DeregisterCriticalServiceAfter` 最小 1 分钟，**不参与摘流判定**；摘流靠发现端
  `?passing=true` 过滤，这一条决定了 RTO 的第一段。
- 进程假死（消息循环卡住但进程还在）由 k8s 的 HTTP `/health` 探针兜底，TTL 只证明「进程活着」。

## 3. 参数与 RTO 预算

| 环节 | 参数 | 位置 |
|---|---|---|
| TTL 判定窗口 | 10s | `websocket.health_check.interval`、`rpc.*.health_check.interval` |
| TTL 上报周期 | 3s（TTL/3） | 代码自动计算 |
| 僵尸注册清理 | 60s | `deregister_after` |
| 发现端刷新 | 5s | `service_center.service_refresh_interval` |
| 空实例集追赶 | 2s | 代码内置 |
| gRPC 调用超时 | 5s | `runtime/src/service_discovery/tonic_service_discovery.rs` |
| 注册中心请求超时 | 3s（限幅 1~10） | `service_center.timeout` |

实测：`kill -9` 摘流 8s、优雅停机摘流 0s；RTO 预算 8s + 5s + 5s = 18s < 30s 目标。

## 4. 摘流与停机顺序

四个进程共用 `runtime/src/shutdown.rs` 的 SIGTERM/SIGINT 处理，顺序固定：

```
收到信号 → 从 Consul 注销 → 停止接收新请求 / 关闭 gRPC → 排空存量 → 停止 TTL 上报 → 退出
```

`msg-gateway` 特殊：**先注销 `ws`，再注销 WebSocket 接入**。反过来会出现
「网关不再接新连接，但 msg-server 还在往里推」的窗口。

`api-gateway` 用 `axum_server::Handle` 做 30s 排空，不依赖 Consul 层面。

## 5. 故障与自愈

| 场景 | 行为 |
|---|---|
| Consul 重启丢状态 | 上报任务每轮先 `GET /v1/agent/service/{id}` 核查，缺失则用原始载荷重建并立即 pass |
| Consul 整体不可达 | 已注册实例的业务不受影响；发现端沿用本地地址集；恢复后自动重新 passing |
| 某个实例 kill -9 | TTL 过期（≤10s）后 `passing=true` 列表里消失，最迟 10+5s 内不再被调用 |
| 发现端拿到空列表 | **保留最后已知良好集合**，不摘光下游；2s 后重试。否则 Consul 抖动会放大成全链路故障 |
| 停机后僵尸实例继续上报 | `stop_ttl_updater` 会停掉该实例的上报任务（句柄存进程级表），否则它会自己续命、永不摘流 |

## 6. 配置

```yaml
service_center:
  host: 127.0.0.1      # 容器内由 CONSUL_HOST 覆盖
  port: 8500           # 容器内由 CONSUL_PORT 覆盖
  timeout: 3
  protocol: http
  service_refresh_interval: 5
```

环境变量：`CONSUL_HOST`、`CONSUL_PORT`、`ADVERTISE_HOST`（见 `config/README.md`）。

## 7. 排查清单

```bash
# 哪些实例当前被判定为健康
curl -s "localhost:8500/v1/health/service/ws?passing=true" | jq '.[].Service | {ID,Address,Port}'

# 单个实例的注册详情（含 TTL 状态）
curl -s localhost:8500/v1/agent/service/ws-<host>-<port> | jq .

# 进程是否还在上报（日志）
grep -E "TTL|Deregister|register_service" logs/msg-server*.log | tail
```

常见现象与原因：

| 现象 | 排查方向 |
|---|---|
| 服务没出现在 `passing` 列表 | 注册地址是不是 `127.0.0.1`；`ADVERTISE_HOST` 是否注入 |
| 停机后实例仍在列表里 | TTL 上报任务没停（看日志有无 `Stopped TTL`）；`deregister_after` 要等 60s |
| 调用方持续 503 | 发现端地址集为空且被清空（检查空集合保护是否生效）；或 gRPC 端口写错 |
| 注册中心恢复后实例不回来 | 看是否有重建注册的日志；`timeout` 是否被填成端口号一类的大值 |
