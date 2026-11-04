# RustIM Kubernetes 编排
#
# 目录内容：
# - 00-namespace.yaml    命名空间
# - 01-secrets.yaml      密钥模板（填值后 kubectl apply，或用 sealed-secrets/external-secrets 替代）
# - postgres.yaml        PostgreSQL StatefulSet
# - redis.yaml           Redis StatefulSet
# - consul.yaml          Consul StatefulSet（服务注册与发现）
# - kafka.yaml           Zookeeper + Kafka 3 broker StatefulSet
# - api-gateway.yaml     HTTP 入口（Deployment + Service + HPA + PDB）
# - msg-gateway.yaml     WebSocket 长连接接入（Deployment + Service + HPA + PDB）
# - msg-server.yaml      消息服务（Deployment + Service + HPA + PDB）
# - relation-service.yaml 关系服务（Deployment + Service + HPA + PDB）
# - oss.yaml             对象存储服务（Deployment + Service + PDB）
#
# 部署顺序（状态依赖）：
#   1. namespace + secrets
#   2. consul / postgres / redis / kafka
#   3. relation-service / msg-server / msg-gateway / oss / api-gateway
#
# 应用配置文件不在此重复维护，直接从仓库 config/config.yaml 生成：
#   kubectl -n rustim create configmap rustim-config --from-file=config/config.yaml --dry-run=client -o yaml | kubectl apply -f -
# （或改用 kustomize configMapGenerator 纳入 CI）
#
# 密钥注入：应用读取 OSS_ACCESS_KEY / OSS_SECRET_KEY / SMS_SECRET_ID /
# SMS_SECRET_KEY / JWT_SECRET 环境变量，见 01-secrets.yaml。
#
# PostgreSQL 高可用：生产建议使用 CloudNativePG / Zalando Postgres Operator，
# 本目录提供单实例 StatefulSet 作为基线。
