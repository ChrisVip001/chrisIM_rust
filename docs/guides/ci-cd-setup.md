# RustIM CI/CD 部署指南

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](../README.md)

## 📋 概述

本文档详细介绍了如何为 RustIM 项目设置完整的 CI/CD 流水线，支持腾讯云 OpenCloudOS 环境下的多环境部署。

## 🏗️ 架构概览

### 分支策略
- **release**: 生产环境分支，自动部署到生产服务器
- **develop**: 测试环境分支，自动部署到测试服务器
- **feature/***: 功能分支，创建 PR 时触发测试

### 部署环境
- **生产环境 (Production)**: 腾讯云 OpenCloudOS 服务器
- **测试环境 (Staging)**: 腾讯云 OpenCloudOS 服务器

## 🚀 快速开始

### 1. 服务器初始化

在腾讯云 OpenCloudOS 服务器上运行初始化脚本：

```bash
# 下载并运行初始化脚本
curl -fsSL https://raw.githubusercontent.com/your-username/rust-im/release/scripts/opencloudos-setup.sh | sudo bash
```

### 2. 配置 GitHub Secrets

在 GitHub 仓库的 Settings > Secrets and variables > Actions 中添加以下密钥：

#### 生产环境密钥
```
SERVER_HOST=your-production-server-ip
SERVER_USER=rustim
SSH_PRIVATE_KEY=your-ssh-private-key
DOCKER_USERNAME=your-docker-hub-username
DOCKER_PASSWORD=your-docker-hub-password
```

#### 测试环境密钥
```
SERVER_HOST_STAGING=your-staging-server-ip
SERVER_USER_STAGING=rustim
SSH_PRIVATE_KEY_STAGING=your-ssh-private-key-staging
```

#### 可选通知密钥
```
SLACK_WEBHOOK=your-slack-webhook-url
```

### 3. 推送代码触发部署

```bash
# 部署到测试环境
git checkout develop
git push origin develop

# 部署到生产环境
git checkout release
git push origin release
```

## 🔧 详细配置

### GitHub Actions 工作流

工作流文件位于 `.github/workflows/ci-cd.yml`，包含以下阶段：

1. **代码质量检查** (`test`)
   - Rust 代码格式检查
   - Clippy 静态分析
   - 单元测试
   - 安全审计

2. **Docker 镜像构建** (`build`)
   - 多架构构建 (amd64, arm64)
   - 镜像缓存优化
   - 自动标签管理

3. **测试环境部署** (`deploy-staging`)
   - 仅在 `develop` 分支触发
   - 自动健康检查

4. **生产环境部署** (`deploy-production`)
   - 仅在 `release` 分支触发
   - 人工审批机制
   - 部署通知

5. **安全扫描** (`security`)
   - Trivy 漏洞扫描
   - SARIF 报告上传

### 多环境配置

#### 测试环境 (Staging)
- **配置文件**: `.env.staging`
- **Docker Compose**: `docker-compose.staging.yml`
- **特点**:
  - 调试模式开启
  - 较小的资源限制
  - 详细日志记录

#### 生产环境 (Production)
- **配置文件**: `.env.production`
- **Docker Compose**: `docker-compose.yml`
- **特点**:
  - 性能优化
  - 安全加固
  - 监控告警

### 部署脚本

#### 主部署脚本
`scripts/deploy-remote.sh` 支持多环境部署：

```bash
# 部署到测试环境
./scripts/deploy-remote.sh -e staging

# 部署到生产环境
./scripts/deploy-remote.sh -e production
```

#### 服务器初始化脚本
`scripts/opencloudos-setup.sh` 专为腾讯云 OpenCloudOS 优化：

- 系统优化配置
- Docker 镜像加速器
- 防火墙配置
- 监控工具安装

## 🔐 安全配置

### SSH 密钥配置

1. **生成 SSH 密钥对**:
```bash
ssh-keygen -t ed25519 -C "rustim-deploy" -f ~/.ssh/rustim_deploy
```

2. **复制公钥到服务器**:
```bash
ssh-copy-id -i ~/.ssh/rustim_deploy.pub rustim@your-server-ip
```

3. **添加私钥到 GitHub Secrets**:
```bash
cat ~/.ssh/rustim_deploy | pbcopy
```

### 环境变量安全

- 生产环境密钥使用强随机字符串
- 数据库密码定期轮换
- JWT 密钥独立生成
- 第三方服务密钥分环境管理

## 📊 监控和日志

### 应用监控

- **Prometheus**: 指标收集
- **Grafana**: 可视化面板
- **Node Exporter**: 系统指标

### 日志管理

- **应用日志**: `/home/rustim/rust-im/logs/`
- **Docker 日志**: 自动轮转
- **系统日志**: journald

### 健康检查

- **API 健康检查**: `GET /health`
- **数据库连接检查**: PostgreSQL
- **缓存连接检查**: Redis
- **消息队列检查**: Kafka

## 🚨 故障排除

### 常见问题

1. **部署失败**
```bash
# 检查服务状态
docker-compose ps

# 查看服务日志
docker-compose logs rustim-api

# 检查系统资源
htop
df -h
```

2. **健康检查失败**
```bash
# 手动健康检查
curl -f http://localhost:8080/health

# 检查端口占用
netstat -tlnp | grep 8080
```

3. **数据库连接问题**
```bash
# 检查数据库状态
docker-compose exec postgres pg_isready -U rustim

# 查看数据库日志
docker-compose logs postgres
```

### 回滚操作

```bash
# 查看可用备份
ls -la /home/rustim/backups/

# 手动回滚到上一个版本
git checkout HEAD~1
./scripts/deploy-remote.sh -e production
```

## 📈 性能优化

### 系统优化

- **文件描述符限制**: 65536
- **网络连接优化**: TCP 参数调优
- **内存管理**: Swap 优化

### Docker 优化

- **镜像缓存**: 多阶段构建
- **资源限制**: CPU 和内存限制
- **网络优化**: 自定义网络配置

### 应用优化

- **连接池**: 数据库连接池优化
- **缓存策略**: Redis 缓存配置
- **异步处理**: Kafka 消息队列

## 🔄 更新和维护

### 定期维护任务

1. **系统更新**:
```bash
sudo yum update -y
```

2. **Docker 清理**:
```bash
docker system prune -f
```

3. **日志清理**:
```bash
find /home/rustim/rust-im/logs/ -name "*.log" -mtime +30 -delete
```

4. **备份验证**:
```bash
ls -la /home/rustim/backups/ | head -10
```

### 版本发布流程

1. **功能开发**: 在 `feature/*` 分支开发
2. **合并测试**: 合并到 `develop` 分支测试
3. **发布准备**: 合并到 `release` 分支
4. **生产部署**: 自动部署到生产环境
5. **版本标记**: 创建 Git 标签

## 📞 支持和联系

如果在部署过程中遇到问题，请：

1. 检查 [故障排除](#故障排除) 部分
2. 查看 GitHub Actions 日志
3. 检查服务器日志文件
4. 提交 GitHub Issue

---

**注意**: 请确保在生产环境中更改所有默认密码和密钥！ 