# RustIM - 云原生即时通讯系统

[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org)
[![Docker](https://img.shields.io/badge/docker-ready-blue.svg)](https://www.docker.com)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

RustIM 是一个基于 Rust 语言开发的高性能、云原生微服务架构即时通讯系统。采用现代化的技术栈，支持大规模并发用户，具备高可用性、可扩展性和容器化部署能力。

## 🚀 核心特性

- **微服务架构**: 模块化设计，服务独立部署和扩展
- **高性能**: 基于 Rust 异步编程，支持百万级并发连接
- **云原生**: 完整的容器化支持，支持 Kubernetes 部署
- **实时通信**: WebSocket 长连接，毫秒级消息推送
- **分布式**: 支持多节点部署，水平扩展
- **安全可靠**: JWT 认证，数据加密传输
- **监控完善**: 集成 Prometheus 监控和链路追踪

## 🏗️ 系统架构

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Web Client    │    │  Mobile Client  │    │  Desktop Client │
└─────────┬───────┘    └─────────┬───────┘    └─────────┬───────┘
          │                      │                      │
          └──────────────────────┼──────────────────────┘
                                 │
                    ┌─────────────┴─────────────┐
                    │      Load Balancer        │
                    └─────────────┬─────────────┘
                                  │
                    ┌─────────────┴─────────────┐
                    │      API Gateway          │
                    │   (认证、路由、限流)        │
                    └─────────────┬─────────────┘
                                  │
        ┌─────────────────────────┼─────────────────────────┐
        │                         │                         │
┌───────┴────────┐    ┌──────────┴───────────┐    ┌────────┴────────┐
│  Message       │    │    Business          │    │   Storage       │
│  Gateway       │    │    Services          │    │   Services      │
│                │    │                      │    │                 │
│ • WebSocket    │    │ • User Service       │    │ • PostgreSQL    │
│ • 消息推送      │    │ • Friend Service     │    │ • Redis         │
│ • 连接管理      │    │ • Group Service      │    │ • Kafka         │
│                │    │ • Message Server     │    │ • OSS           │
└────────────────┘    └──────────────────────┘    └─────────────────┘
```

### 服务组件

| 服务名称 | 端口 | 功能描述 |
|---------|------|----------|
| **api-gateway** | 8080 | API网关，统一入口，认证授权，路由转发 |
| **msg-gateway** | 8085 | 消息网关，WebSocket连接管理，实时消息推送 |
| **user-service** | 50001 | 用户管理，注册登录，用户信息维护 |
| **friend-service** | 50002 | 好友关系管理，好友申请，黑名单 |
| **group-service** | 50003 | 群组管理，群成员管理，群权限控制 |
| **msg-server** | 50004 | 消息处理，消息存储，消息分发 |
| **oss** | 50005 | 对象存储服务，文件上传下载 |

## 🛠️ 技术栈

### 后端技术
- **语言**: Rust 1.75+
- **异步运行时**: Tokio
- **Web框架**: Axum (HTTP), Tonic (gRPC)
- **数据库**: PostgreSQL, Redis, MongoDB
- **消息队列**: Apache Kafka
- **认证**: JWT
- **监控**: Prometheus, Jaeger
- **配置管理**: YAML/TOML/JSON

### 基础设施
- **容器化**: Docker, Docker Compose
- **编排**: Kubernetes (可选)
- **负载均衡**: Nginx (可选)
- **服务发现**: Consul (可选)
- **日志收集**: ELK Stack (可选)

## 📋 环境要求

### 开发环境
- Rust 1.75+
- Docker 20.10+
- Docker Compose 2.0+
- Git

### 生产环境
- 4 Core CPU, 8GB RAM (最小配置)
- 100GB 存储空间
- Docker 或 Kubernetes 环境

## 🚀 快速开始

### 0. 安装 Docker 环境 (必需)

如果服务器还没有安装 Docker，请先运行安装脚本：

```bash
# 克隆项目
git clone https://github.com/yourusername/rust-im.git
cd rust-im

# 安装 Docker 环境 (支持 Ubuntu/Debian/CentOS/RHEL)
chmod +x scripts/install-docker.sh
./scripts/install-docker.sh

# 安装完成后，重新登录或运行以下命令使 docker 组权限生效
newgrp docker

# 验证 Docker 安装
docker --version
docker-compose --version
docker run hello-world
```

**支持的操作系统:**
- Ubuntu 18.04+
- Debian 10+
- CentOS 7+
- RHEL 7+
- Rocky Linux 8+
- AlmaLinux 8+

### 1. 克隆项目

```bash
git clone https://github.com/yourusername/rust-im.git
cd rust-im
```

### 2. 环境配置

```bash
# 复制环境变量文件
cp .env.example .env

# 编辑配置文件
vim .env
```

### 3. 一键部署

```bash
# 使用 Docker Compose 部署
chmod +x scripts/deploy.sh
./scripts/deploy.sh

# 或者手动部署
docker-compose up -d
```

### 4. 验证部署

```bash
# 检查服务状态
docker-compose ps

# 查看日志
docker-compose logs -f

# 健康检查
curl http://localhost:8080/health
```

## 🔧 配置说明

### 环境变量

| 变量名 | 默认值 | 说明 |
|--------|--------|------|
| `DATABASE_URL` | - | PostgreSQL 连接字符串 |
| `REDIS_URL` | redis://localhost:6379 | Redis 连接地址 |
| `KAFKA_BROKERS` | localhost:9092 | Kafka 集群地址 |
| `JWT_SECRET` | - | JWT 签名密钥 |
| `LOG_LEVEL` | info | 日志级别 |

### 配置文件

主配置文件位于 `config/config.yaml`，支持以下配置：

- **数据库配置**: PostgreSQL, Redis, MongoDB 连接参数
- **服务配置**: 各微服务的监听地址和端口
- **认证配置**: JWT 密钥、过期时间等
- **限流配置**: API 限流规则
- **监控配置**: Prometheus 指标暴露

## 🐳 Docker 部署

### 构建镜像

```bash
# 构建所有服务镜像
docker build -t rustim:latest .

# 或使用多阶段构建
docker build --target production -t rustim:prod .
```

### 使用 Docker Compose

```bash
# 启动所有服务
docker-compose up -d

# 启动特定服务
docker-compose up -d postgres redis kafka

# 查看服务状态
docker-compose ps

# 停止所有服务
docker-compose down
```

### 生产环境部署

```bash
# 使用生产配置
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d

# 启用监控
docker-compose -f docker-compose.yml -f docker-compose.telemetry.yml up -d
```

## ☸️ Kubernetes 部署

### 前置条件

```bash
# 安装 kubectl
curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"

# 安装 Helm (可选)
curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
```

### 部署步骤

```bash
# 创建命名空间
kubectl create namespace rustim

# 部署基础设施
kubectl apply -f k8s/infrastructure/

# 部署应用服务
kubectl apply -f k8s/services/

# 检查部署状态
kubectl get pods -n rustim
```

## 📊 监控和运维

### 健康检查

```bash
# API 网关健康检查
curl http://localhost:8080/health

# 各服务健康检查
curl http://localhost:8080/api/users/health
curl http://localhost:8080/api/friends/health
curl http://localhost:8080/api/groups/health
```

### 监控指标

访问 Prometheus 指标端点：
- API Gateway: http://localhost:8080/metrics
- 各微服务: http://localhost:PORT/metrics

### 日志查看

```bash
# 查看所有服务日志
docker-compose logs -f

# 查看特定服务日志
docker-compose logs -f api-gateway
docker-compose logs -f user-service

# 实时跟踪日志
docker-compose logs -f --tail=100 msg-gateway
```

## 🧪 测试

### 单元测试

```bash
# 运行所有测试
cargo test

# 运行特定服务测试
cargo test -p user-service
cargo test -p api-gateway
```

### 集成测试

```bash
# 启动测试环境
docker-compose -f docker-compose.test.yml up -d

# 运行集成测试
cargo test --test integration

# 性能测试
./scripts/benchmark.sh
```

### API 测试

```bash
# 使用 curl 测试
curl -X POST http://localhost:8080/api/users/register \
  -H "Content-Type: application/json" \
  -d '{"username":"test","password":"123456","email":"test@example.com"}'

# 使用 Postman 集合
# 导入 docs/postman/RustIM.postman_collection.json
```

## 🔒 安全

### 认证授权
- JWT Token 认证
- 角色权限控制
- API 限流保护

### 数据安全
- 密码 bcrypt 加密
- HTTPS/WSS 传输加密
- 敏感数据脱敏

### 网络安全
- 防火墙配置
- IP 白名单
- DDoS 防护

## 📈 性能优化

### 数据库优化
- 连接池配置
- 索引优化
- 读写分离

### 缓存策略
- Redis 缓存热点数据
- 本地缓存减少网络开销
- CDN 加速静态资源

### 消息队列
- Kafka 异步处理
- 消息分区提高并发
- 消费者组负载均衡

## 🛠️ 开发指南

### 代码结构

```
rust-im/
├── api-gateway/          # API网关服务
├── msg-gateway/          # 消息网关服务
├── user-service/         # 用户服务
├── friend-service/       # 好友服务
├── group-service/        # 群组服务
├── msg-server/           # 消息服务
├── oss/                  # 对象存储服务
├── common/               # 共享代码库
├── cache/                # 缓存模块
├── config/               # 配置文件
├── scripts/              # 部署脚本
├── docs/                 # 文档
└── k8s/                  # Kubernetes 配置
```

### 添加新服务

1. 创建服务目录和 Cargo.toml
2. 实现服务逻辑
3. 添加到 workspace
4. 更新 Docker 配置
5. 添加路由配置

### 代码规范

```bash
# 代码格式化
cargo fmt

# 代码检查
cargo clippy

# 安全审计
cargo audit
```

## 🤝 贡献指南

1. Fork 项目
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## 📄 许可证

本项目采用 MIT 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🆘 支持

- 📧 邮箱: support@rustim.com
- 💬 QQ群: 123456789
- 📖 文档: https://docs.rustim.com
- 🐛 问题反馈: [GitHub Issues](https://github.com/yourusername/rust-im/issues)

## 🗺️ 路线图

- [ ] 支持音视频通话
- [ ] 移动端 SDK
- [ ] 消息加密
- [ ] 多租户支持
- [ ] AI 智能助手
- [ ] 区块链集成

---

⭐ 如果这个项目对你有帮助，请给我们一个 Star！
