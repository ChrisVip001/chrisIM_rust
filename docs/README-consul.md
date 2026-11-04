# Consul 服务注册与发现集成方案

## 概述

本文档描述了在 IM 系统中集成 Consul 实现服务注册与发现的方案。通过 Consul，我们实现了服务的自动注册、健康检查和服务发现，提高了系统的可靠性和弹性。

## 架构设计

整个服务注册与发现架构包含以下组件：

1. **Consul 服务器**：作为服务注册中心，存储所有服务实例信息
2. **ServiceRegistry**：通用库，提供服务注册和发现功能
3. **健康检查端点**：每个微服务提供 HTTP 健康检查接口
4. **负载均衡**：API 网关通过轮询算法实现对后端服务的负载均衡

## 技术实现

### 1. ServiceRegistry 通用库

在 `common/src/service_registry.rs` 中实现通用的服务注册与发现功能：

```rust
pub struct ServiceRegistry {
    http_client: Client,
    consul_url: String,
    service_id: Option<String>,
}
```

主要功能包括：
- `register_service`：注册服务到 Consul
- `deregister_service`：从 Consul 注销服务
- `discover_service`：发现特定服务的所有实例

### 2. 微服务集成

每个微服务的集成流程：

1. 在主程序中创建 ServiceRegistry 实例
2. 注册当前服务到 Consul，指定健康检查路径
3. 启动一个单独的 HTTP 服务器用于健康检查
4. 实现优雅关闭逻辑，包括从 Consul 注销

```rust
// 创建并注册到Consul
let mut service_registry = ServiceRegistry::from_env();
let service_id = service_registry.register_service(
    "service-name",
    host,
    port + 1, // 健康检查端口
    vec!["tag1".to_string(), "tag2".to_string()],
    "/health",
    "15s",
).await?;
```

### 3. API 网关负载均衡

API 网关使用 LoadBalancer 实现对后端服务的负载均衡：

```rust
pub struct LoadBalancer {
    instances: Arc<Mutex<Vec<Channel>>>,
    next_index: Arc<AtomicUsize>,
    service_type: ServiceType,
}
```

关键实现：
- 定期从 Consul 获取服务实例列表
- 使用轮询算法选择服务实例
- 处理服务实例不可用的情况

### 4. 配置说明

系统使用以下环境变量进行配置：

| 变量名 | 说明 | 默认值 |
|--------|------|--------|
| CONSUL_URL | Consul 服务器地址 | http://localhost:8500 |
| SERVICE_REFRESH_INTERVAL | 服务刷新间隔（秒） | 30 |

## 部署说明

### 前置条件

1. 部署 Consul 服务器
```
docker run -d -p 8500:8500 -p 8600:8600/udp consul
```

2. 确保微服务的健康检查端口可访问
3. 配置合适的环境变量

### 监控和管理

服务注册后，可以通过 Consul UI 进行监控和管理：
- 访问 http://localhost:8500/ui/
- 查看服务注册状态和健康检查结果
- 进行服务维护操作

## 错误处理

1. 服务注册失败：检查 Consul 服务器是否可访问
2. 健康检查失败：检查微服务健康检查端点是否正常响应
3. 服务发现返回空列表：确认服务名称是否正确，以及是否有健康实例

## 最佳实践

1. 为每个服务配置唯一的服务 ID
2. 使用合适的健康检查间隔，避免过于频繁的检查
3. 合理设置 DeregisterCriticalServiceAfter 值，清理失败的服务实例
4. 实现优雅关闭，确保服务在停止前从 Consul 注销
5. 日志记录服务注册和发现的关键操作 