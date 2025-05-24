# RustIM Kubernetes 部署指南

## 概述

本文档描述了如何在 Kubernetes 集群中部署 RustIM 即时通讯系统。

## 架构概览

RustIM 系统包含以下组件：

### 核心服务
- **API Gateway**: HTTP API 网关 (端口 8080)
- **Message Gateway**: WebSocket 消息网关 (端口 8085)
- **User Service**: 用户管理服务 (端口 8081)
- **Friend Service**: 好友关系服务 (端口 8082)
- **Group Service**: 群组管理服务 (端口 8083)
- **Message Server**: 消息处理服务 (端口 8084)
- **OSS Service**: 对象存储服务 (端口 8086)

### 基础设施
- **PostgreSQL**: 主数据库 (端口 5432)
- **Redis**: 缓存和会话存储 (端口 6379)
- **Kafka**: 消息队列 (端口 9092)
- **Zookeeper**: Kafka 协调服务 (端口 2181)

### 监控组件
- **Prometheus**: 指标收集 (端口 9090)
- **Grafana**: 监控仪表板 (端口 3000)
- **Alertmanager**: 告警管理 (端口 9093)

## 前置要求

### 集群要求
- Kubernetes 1.20+
- 至少 4 CPU 核心
- 至少 8GB 内存
- 50GB 存储空间

### 必需组件
- kubectl 客户端
- Helm 3.0+ (可选)
- cert-manager (用于 SSL 证书)
- nginx-ingress-controller

## 快速开始

### 1. 克隆项目
```bash
git clone https://github.com/your-org/rust-im.git
cd rust-im
```

### 2. 构建镜像
```bash
# 构建所有服务镜像
docker build -t rustim/api-gateway:latest --target runtime .
docker build -t rustim/msg-gateway:latest --target runtime .
# ... 其他服务
```

### 3. 部署到 Kubernetes
```bash
# 使用部署脚本
./scripts/k8s-deploy.sh deploy --wait

# 或手动部署
kubectl apply -f k8s/namespace.yaml
kubectl apply -f k8s/configmap.yaml
kubectl apply -f k8s/secrets.yaml
kubectl apply -f k8s/
```

### 4. 验证部署
```bash
# 检查服务状态
./scripts/k8s-deploy.sh status

# 查看 Pod 状态
kubectl get pods -n rustim

# 查看服务
kubectl get svc -n rustim
```

## 详细部署步骤

### 1. 准备环境

#### 安装 cert-manager
```bash
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.12.0/cert-manager.yaml
```

#### 安装 nginx-ingress
```bash
kubectl apply -f https://raw.githubusercontent.com/kubernetes/ingress-nginx/controller-v1.8.1/deploy/static/provider/cloud/deploy.yaml
```

### 2. 配置存储

#### 创建持久卷 (如果需要)
```yaml
apiVersion: v1
kind: PersistentVolume
metadata:
  name: postgres-pv
spec:
  capacity:
    storage: 20Gi
  accessModes:
    - ReadWriteOnce
  persistentVolumeReclaimPolicy: Retain
  storageClassName: local-storage
  local:
    path: /data/postgres
  nodeAffinity:
    required:
      nodeSelectorTerms:
      - matchExpressions:
        - key: kubernetes.io/hostname
          operator: In
          values:
          - your-node-name
```

### 3. 配置密钥

#### 更新 Secrets
```bash
# 编辑 secrets.yaml 文件，更新以下值：
# - postgres-password: 数据库密码 (base64 编码)
# - jwt-secret: JWT 密钥 (base64 编码)
# - aws-access-key-id: AWS 访问密钥 (base64 编码)
# - aws-secret-access-key: AWS 密钥 (base64 编码)

# 生成 base64 编码
echo -n "your-password" | base64
```

### 4. 配置域名

#### 更新 Ingress 配置
编辑 `k8s/ingress.yaml`，将域名替换为你的实际域名：
- `api.rustim.com` -> `api.yourdomain.com`
- `ws.rustim.com` -> `ws.yourdomain.com`
- `monitor.rustim.com` -> `monitor.yourdomain.com`

#### DNS 配置
确保域名指向你的 Kubernetes 集群的 LoadBalancer IP：
```bash
# 获取 LoadBalancer IP
kubectl get svc -n ingress-nginx ingress-nginx-controller
```

### 5. 部署服务

#### 按顺序部署
```bash
# 1. 基础设施
kubectl apply -f k8s/namespace.yaml
kubectl apply -f k8s/configmap.yaml
kubectl apply -f k8s/secrets.yaml

# 2. 数据库和缓存
kubectl apply -f k8s/postgres.yaml
kubectl apply -f k8s/redis.yaml
kubectl apply -f k8s/kafka.yaml

# 3. 等待基础设施就绪
kubectl wait --for=condition=ready pod -l app=postgres -n rustim --timeout=300s
kubectl wait --for=condition=ready pod -l app=redis -n rustim --timeout=300s
kubectl wait --for=condition=ready pod -l app=kafka -n rustim --timeout=300s

# 4. 应用服务
kubectl apply -f k8s/api-gateway.yaml
kubectl apply -f k8s/msg-gateway.yaml
kubectl apply -f k8s/user-service.yaml
kubectl apply -f k8s/friend-service.yaml
kubectl apply -f k8s/group-service.yaml
kubectl apply -f k8s/msg-server.yaml
kubectl apply -f k8s/oss.yaml

# 5. 监控组件
kubectl apply -f k8s/monitoring.yaml

# 6. Ingress
kubectl apply -f k8s/ingress.yaml
```

## 配置说明

### 环境变量配置

#### 应用配置
- `RUST_LOG`: 日志级别 (info, debug, warn, error)
- `REDIS_URL`: Redis 连接地址
- `KAFKA_BROKERS`: Kafka 集群地址

#### 数据库配置
- `POSTGRES_HOST`: PostgreSQL 主机
- `POSTGRES_PORT`: PostgreSQL 端口
- `POSTGRES_DB`: 数据库名称
- `POSTGRES_USER`: 数据库用户
- `POSTGRES_PASSWORD`: 数据库密码 (来自 Secret)

#### 服务端口配置
- `API_GATEWAY_PORT`: API 网关端口
- `MSG_GATEWAY_PORT`: 消息网关端口
- `USER_SERVICE_PORT`: 用户服务端口
- `FRIEND_SERVICE_PORT`: 好友服务端口
- `GROUP_SERVICE_PORT`: 群组服务端口
- `MSG_SERVER_PORT`: 消息服务端口
- `OSS_PORT`: OSS 服务端口

### 资源配置

#### CPU 和内存限制
```yaml
resources:
  requests:
    cpu: 100m
    memory: 256Mi
  limits:
    cpu: 500m
    memory: 512Mi
```

#### 自动扩缩容
```yaml
spec:
  minReplicas: 2
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
```

## 监控和日志

### 访问监控界面

#### Grafana
- URL: https://monitor.yourdomain.com/grafana
- 用户名: admin
- 密码: admin123

#### Prometheus
- URL: https://monitor.yourdomain.com/prometheus

#### Alertmanager
- URL: https://monitor.yourdomain.com/alertmanager

### 查看日志
```bash
# 查看特定服务日志
./scripts/k8s-deploy.sh logs --service api-gateway

# 实时跟踪日志
./scripts/k8s-deploy.sh logs --service api-gateway --follow

# 查看所有服务日志
kubectl logs -n rustim --all-containers=true --tail=100
```

### 监控指标

#### 应用指标
- HTTP 请求数量和延迟
- WebSocket 连接数
- 数据库连接池状态
- 消息队列积压

#### 系统指标
- CPU 使用率
- 内存使用率
- 磁盘 I/O
- 网络流量

## 运维操作

### 扩缩容
```bash
# 扩展 API 网关到 5 个实例
./scripts/k8s-deploy.sh scale --service api-gateway --replicas 5

# 或使用 kubectl
kubectl scale deployment api-gateway --replicas=5 -n rustim
```

### 滚动更新
```bash
# 更新镜像
kubectl set image deployment/api-gateway api-gateway=rustim/api-gateway:v1.1.0 -n rustim

# 查看更新状态
kubectl rollout status deployment/api-gateway -n rustim

# 回滚更新
kubectl rollout undo deployment/api-gateway -n rustim
```

### 备份和恢复

#### 数据库备份
```bash
# 创建备份 Job
kubectl create job --from=cronjob/postgres-backup postgres-backup-manual -n rustim

# 手动备份
kubectl exec -it postgres-0 -n rustim -- pg_dump -U rustim rustim > backup.sql
```

#### 配置备份
```bash
# 导出配置
kubectl get configmap,secret -n rustim -o yaml > rustim-config-backup.yaml
```

## 故障排除

### 常见问题

#### Pod 无法启动
```bash
# 查看 Pod 状态
kubectl describe pod <pod-name> -n rustim

# 查看 Pod 日志
kubectl logs <pod-name> -n rustim

# 查看事件
kubectl get events -n rustim --sort-by='.lastTimestamp'
```

#### 服务无法访问
```bash
# 检查服务
kubectl get svc -n rustim

# 检查端点
kubectl get endpoints -n rustim

# 测试服务连通性
kubectl run test-pod --image=busybox -it --rm -- wget -qO- http://api-gateway-service:8080/health
```

#### 数据库连接问题
```bash
# 检查数据库状态
kubectl exec -it postgres-0 -n rustim -- psql -U rustim -c "SELECT version();"

# 检查网络策略
kubectl get networkpolicy -n rustim

# 测试数据库连接
kubectl run test-db --image=postgres:16-alpine -it --rm -- psql -h postgres-service -U rustim -d rustim
```

### 性能调优

#### 数据库优化
```sql
-- 查看慢查询
SELECT query, mean_time, calls 
FROM pg_stat_statements 
ORDER BY mean_time DESC 
LIMIT 10;

-- 查看连接数
SELECT count(*) FROM pg_stat_activity;
```

#### Redis 优化
```bash
# 查看 Redis 信息
kubectl exec -it redis-0 -n rustim -- redis-cli info memory

# 查看慢日志
kubectl exec -it redis-0 -n rustim -- redis-cli slowlog get 10
```

## 安全配置

### 网络策略
- 限制 Pod 间通信
- 只允许必要的端口访问
- 隔离不同环境

### RBAC 配置
```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  namespace: rustim
  name: rustim-operator
rules:
- apiGroups: [""]
  resources: ["pods", "services"]
  verbs: ["get", "list", "watch"]
```

### 密钥管理
- 使用 Kubernetes Secrets 存储敏感信息
- 定期轮换密钥
- 启用静态加密

## 升级指南

### 版本升级
1. 备份当前配置和数据
2. 更新镜像标签
3. 执行滚动更新
4. 验证服务功能
5. 监控系统状态

### 配置更新
1. 更新 ConfigMap 或 Secret
2. 重启相关 Pod
3. 验证配置生效

## 参考资料

- [Kubernetes 官方文档](https://kubernetes.io/docs/)
- [Prometheus 监控指南](https://prometheus.io/docs/)
- [Grafana 配置文档](https://grafana.com/docs/)
- [cert-manager 使用指南](https://cert-manager.io/docs/)
- [nginx-ingress 配置](https://kubernetes.github.io/ingress-nginx/) 