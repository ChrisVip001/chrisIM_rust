# RustIM Kubernetes 编排

目录内容：

- `00-namespace.yaml` 命名空间
- `01-secrets.yaml` 密钥模板（填值后 `kubectl apply`，或用 sealed-secrets/external-secrets 替代）
- `consul.yaml` Consul StatefulSet（服务注册与发现）
- `postgres.yaml` / `redis.yaml` PostgreSQL、Redis StatefulSet
- `kafka.yaml` Zookeeper + Kafka 3 broker StatefulSet
- `api-gateway.yaml` HTTP 入口（Deployment + Service + HPA + PDB）
- `msg-gateway.yaml` WebSocket 长连接接入
- `msg-server.yaml` 消息服务
- `relation-service.yaml` 关系服务

`oss` 是库 crate（被 api-gateway 直接链接），没有独立二进制，因此不部署。

部署顺序（状态依赖）：

1. namespace + secrets
2. consul / postgres / redis / kafka
3. relation-service / msg-server / msg-gateway / api-gateway

配置文件不在此重复维护，直接从仓库生成：

```bash
kubectl -n rustim create configmap rustim-config --from-file=config/config.yaml --dry-run=client -o yaml | kubectl apply -f -
```

业务容器必须注入两个地址变量（yaml 中已通过 `fieldRef` 与 `value` 配好）：

- `CONSUL_HOST=consul.rustim.svc.cluster.local`：配置里的 `127.0.0.1` 在 Pod 内指向自己。
- `ADVERTISE_HOST=status.podIP`：注册到 Consul 的地址必须是 Pod IP，否则服务发现会把流量导向调用方自己；监听地址仍用配置中的 host。

密钥注入：`OSS_ACCESS_KEY` / `OSS_SECRET_KEY` / `SMS_SECRET_ID` / `SMS_SECRET_KEY` / `JWT_SECRET`，见 `01-secrets.yaml`。

## 可用性与 RTO 预算（阶段 1）

- `kill -9` 后 Consul TTL 窗口 10s 判定 critical，发现端 5s 刷新一次 → 摘流 ≤ 15s；k8s readiness 5s×2 摘除端点，副本由 Deployment 重建 → RTO 预算 30s。
- `DeregisterCriticalServiceAfter=60s` 只用于清理僵尸注册（Consul 最小 1m），不参与摘流；摘流靠 `?passing=true` 过滤。
- TTL 上报周期为窗口的 1/3（10s → 3s），注销时能停掉上报任务，避免停机后的僵尸实例被重新注册。
- 探针：readiness 5s / 2 次失败，liveness 15s / 3 次失败；`terminationGracePeriodSeconds=30` 与 RTO 预算对齐。

单点决策（本基线）：

- Kafka 3 broker（`min.insync.replicas=2`）；Consul / Redis / PostgreSQL 单实例。
- Consul 不在请求关键路径：发现端在注册中心返回空实例集时保留最后已知集合，TTL 上报任务在 Consul 重启后重建注册，因此短时不可用不会断流。代价是故障期间新实例无法注册、地址变更不生效。
- 生产应外接 Consul 集群、Redis Sentinel/Cluster 与 PG Operator（CloudNativePG / Zalando），本目录只提供可运行基线。
- 需在真实集群验证：Pod 重建耗时、Consul 重启后的注册重建、多实例负载接管；本机验收网只覆盖摘流时间（`tests/faults/rto-consul.sh`）。

PostgreSQL 高可用：生产建议使用 CloudNativePG / Zalando Postgres Operator，本目录提供单实例 StatefulSet 作为基线。
