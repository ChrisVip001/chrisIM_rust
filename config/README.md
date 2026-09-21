# 全局配置加载器

本项目实现了一个强大的配置系统，支持全局配置和服务特定配置的智能合并。

## 特性

- **分层配置**：支持全局配置和服务特定配置
- **智能合并**：服务特定配置可以覆盖全局配置中的特定字段，而不需要复制全部配置
- **全局单例**：使用静态全局配置单例，方便在代码任何地方访问
- **配置热重载**：支持配置文件变更监控和自动重新加载（需启用 `dynamic-config` 特性）
- **环境变量支持**：配置可以通过环境变量覆盖

## 目录结构

```
config/
  ├── config.yaml         # 全局配置文件
  ├── services/           # 服务特定配置目录
  │   ├── api-gateway.yaml
  │   ├── user-service.yaml
  │   ├── friend-service.yaml
  │   ├── group-service.yaml
  │   ├── msg-server.yaml
  │   └── msg-gateway.yaml
  ├── README.md           # 本文档
  └── ...
```

## 使用方法

### 1. 基本用法

在服务启动时初始化配置：

```rust
use common::config::{Component, ConfigLoader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化服务配置（自动合并全局配置和服务特定配置）
    let mut loader = ConfigLoader::new(Component::UserServer);
    let config = loader.load()?;
    
    // 使用配置
    println!("服务器端口: {}", config.server.port);
    println!("数据库连接: {}@{}", config.database.postgres.user, config.database.postgres.host);
    
    Ok(())
}
```

### 2. 使用全局配置单例

初始化全局配置单例以便在代码的任何地方访问配置：

```rust
// 在应用启动时初始化
ConfigLoader::init_global()?;

// 在代码任何地方访问
if let Some(config) = ConfigLoader::get_global() {
    let redis_url = config.redis.url();
    // ...
}
```

### 3. 监控配置文件变更

启用配置文件监控以支持配置热重载（需要 `dynamic-config` 特性）：

```rust
#[cfg(feature = "dynamic-config")]
ConfigLoader::watch_config_changes(Component::UserServer)?;
```

当配置文件变更时，系统会自动重新加载配置并更新全局单例。

### 4. 服务配置示例

在 `config/services/` 目录下创建与服务对应的配置文件，只需要包含你想要覆盖的字段，例如：

```yaml
# config/services/user-service.yaml
component: UserServer

server:
  port: 50002  # 覆盖全局配置中的端口

database:
  postgres:
    user: user_service_user
    password: user_service_password
    database: user_db
```

## 开启动态配置监控

在 `Cargo.toml` 中启用 `dynamic-config` 特性：

```toml
[dependencies]
common = { path = "../common", features = ["dynamic-config"] }
```

## 贡献

欢迎提交 Pull Request 以改进配置系统。 