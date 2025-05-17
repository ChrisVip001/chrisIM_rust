# RustIM 日志配置指南

本文档介绍如何在 RustIM 微服务中配置日志输出级别，包括通过配置文件和环境变量两种方式。

## 日志级别说明

RustIM 使用 `tracing` 和 `tracing-subscriber` 库进行日志管理，支持以下日志级别（从详细到简略）：

- **TRACE**: 最详细的日志，包含程序执行的每一步
- **DEBUG**: 调试信息，对开发和排错有帮助
- **INFO**: 普通信息，应用的正常状态和重要事件
- **WARN**: 警告信息，表示可能的问题但不影响程序继续运行
- **ERROR**: 错误信息，表示发生了严重问题

## 配置文件方式

RustIM 支持通过 YAML、JSON 或 TOML 格式的配置文件设置日志级别。以下是一个 YAML 格式的配置示例：

```yaml
log:
  # 全局日志级别: trace, debug, info, warn, error
  level: "info"
  # 日志输出位置: console, file
  output: "console"
  # SQLx库的日志级别，用于控制数据库查询日志
  sqlx_level: "debug"
  # 其他组件的日志级别配置
  components:
    tower: "info"     # HTTP服务器中间件
    hyper: "info"     # HTTP客户端/服务器
    axum: "debug"     # Axum Web框架
    tonic: "info"     # gRPC框架
    rustIM: "debug"   # 自定义应用组件
```

将此配置文件保存为 `config/config.yaml`，微服务启动时会自动加载。

## 环境变量方式

RustIM 支持通过环境变量覆盖配置文件中的设置，环境变量的优先级高于配置文件。

### 全局日志级别

使用 `RUST_LOG` 环境变量设置全局日志级别：

```bash
# 设置全局日志级别为DEBUG
export RUST_LOG=debug

# 启动服务
./user-service
```

### 组件特定日志级别

可以使用 `RUST_LOG` 环境变量的过滤器语法设置特定组件的日志级别：

```bash
# 设置不同组件的日志级别
export RUST_LOG=info,sqlx=debug,tower=warn

# 启动服务
./user-service
```

此外，RustIM 还支持使用特定组件的环境变量：

```bash
# 只设置 SQLx 的日志级别
export RUST_LOG_SQLX=debug

# 设置 Hyper 的日志级别
export RUST_LOG_HYPER=info

# 启动服务
./user-service
```

### 常用环境变量

以下是一些常用的环境变量：

| 环境变量             | 作用                   | 示例值               |
|------------------|----------------------|-------------------|
| `RUST_LOG`       | 设置全局日志级别和过滤器         | `info,sqlx=debug` |
| `RUST_LOG_SQLX`  | 设置 SQLx 日志级别         | `debug`           |
| `RUST_LOG_TOWER` | 设置 Tower 中间件日志级别     | `info`            |
| `RUST_LOG_HYPER` | 设置 Hyper HTTP 库日志级别  | `info`            |
| `RUST_LOG_AXUM`  | 设置 Axum Web 框架日志级别   | `debug`           |
| `RUST_LOG_TONIC` | 设置 Tonic gRPC 框架日志级别 | `info`            |

## 在 Docker 中设置日志级别

在 Docker 环境中，可以通过环境变量轻松设置日志级别：

```yaml
# docker-compose.yml 示例
version: '3'
services:
  user-service:
    image: rustim/user-service
    environment:
      - RUST_LOG=info,sqlx=debug
      - RUST_LOG_TOWER=warn
```

## 在开发环境中使用

在开发环境中，通常需要更详细的日志信息帮助调试：

```bash
# 使用详细日志进行开发调试
export RUST_LOG=debug,sqlx=trace
cargo run --bin user-service
```

## 在生产环境中使用

在生产环境中，通常使用更简练的日志级别避免性能影响：

```bash
# 生产环境推荐设置
export RUST_LOG=info,sqlx=warn
./user-service
```

## 排错技巧

如果需要排查 SQL 查询相关问题，可以临时提高 SQLx 的日志级别：

```bash
# 临时提高 SQLx 日志级别来排查数据库问题
export RUST_LOG_SQLX=trace
./user-service
```

如果需要排查 HTTP 相关问题，可以提高 Hyper 和 Tower 的日志级别：

```bash
# 提高 HTTP 相关组件的日志级别
export RUST_LOG_HYPER=debug
export RUST_LOG_TOWER=debug
./user-service
``` 