# RustIM 日志聚合与分布式链路追踪

本文档提供了在 RustIM 微服务架构中设置和使用日志聚合与分布式链路追踪的详细指南。

## 目录

- [快速开始](#快速开始)
- [系统架构](#系统架构)
- [配置指南](#配置指南)
- [查看和分析数据](#查看和分析数据)
- [故障排除](#故障排除)
- [扩展功能](#扩展功能)

## 快速开始

### 1. 前置条件

- 安装 Docker 和 Docker Compose（版本至少为 19.03+）
- 克隆 RustIM 代码库
- 确保有足够的硬盘空间（至少 4GB 推荐）

### 2. 启动监控系统

```bash
# 启动日志聚合和链路追踪系统
docker-compose -f docker-compose.telemetry.yml up -d

# 检查服务状态
docker-compose -f docker-compose.telemetry.yml ps
```

### 3. 配置微服务

确保在 `config/config.yaml` 中启用了链路追踪功能：

```yaml
log:
  level: "info"
  output: "console"
  format: "json"  # 使用json格式输出日志，便于ELK收集
  sqlx_level: "debug"
  components:
    tower: "warn"
    hyper: "warn"
    rustIM: "debug"

telemetry:
  enabled: true                       # 启用链路追踪
  endpoint: "http://localhost:4317"   # Jaeger OTLP 端点
  sampling_ratio: 1.0                 # 采样率
  propagation: "tracecontext"         # 传播方式
```

### 4. 访问监控服务

- Kibana (日志查询与分析): http://localhost:5601
- Jaeger UI (链路追踪查看): http://localhost:16686
- Prometheus (指标监控): http://localhost:9090
- Grafana (指标可视化): http://localhost:3000

## 系统架构

RustIM 监控系统由以下组件组成：

### 日志聚合系统

- **Elasticsearch**: 存储和索引日志数据
- **Logstash**: 接收、处理和转发日志数据
- **Kibana**: 提供日志数据的可视化和查询界面
- **Filebeat**: 从日志文件收集日志并转发到 Logstash

### 分布式链路追踪系统

- **OpenTelemetry**: 在应用程序中集成的链路追踪SDK
- **Jaeger**: 存储、处理和可视化链路追踪数据

### 指标监控系统

- **Prometheus**: 收集和存储指标数据
- **Grafana**: 提供指标数据的可视化和仪表板

## 配置指南

### 日志配置

RustIM 微服务使用 `tracing` 和 `tracing-subscriber` 进行日志记录。主要配置位于 `config/config.yaml`：

```yaml
log:
  level: "info"            # 全局日志级别
  output: "console"        # 输出位置
  format: "json"           # 格式: json 或 plain
  sqlx_level: "debug"      # SQL 查询日志级别
  components:              # 各组件日志级别
    tower: "warn"
    hyper: "warn"
    rustIM: "debug"
```

特别说明：
- 在生产环境中，将 `format` 设置为 `json` 以便于日志聚合
- 使用 `components` 精细控制各组件的日志级别

### 链路追踪配置

```yaml
telemetry:
  enabled: true                      # 是否启用链路追踪
  endpoint: "http://localhost:4317"  # Jaeger OTLP 端点
  sampling_ratio: 1.0                # 采样率 (0.0-1.0)
  propagation: "tracecontext"        # 上下文传播方式
```

特别说明：
- 在高流量生产环境中，建议将 `sampling_ratio` 设置为较小值（如 0.1）
- `endpoint` 应设置为实际 Jaeger 服务的地址

### 环境变量覆盖

可以使用环境变量覆盖配置文件中的设置：

```bash
# 日志级别设置
export RUST_LOG=info
export RUST_LOG_SQLX=debug
export LOG_FORMAT=json

# 链路追踪设置
export TELEMETRY_ENABLED=true
export TELEMETRY_ENDPOINT=http://jaeger:4317
export TELEMETRY_SAMPLING_RATIO=0.1
```

## 查看和分析数据

### 日志查询（Kibana）

1. 访问 Kibana: http://localhost:5601
2. 创建索引模式: Management > Stack Management > Kibana > Index Patterns
   - 索引模式: `rustim-*`
   - 时间字段: `@timestamp`
3. 使用 Discover 查询日志:
   - 按服务名过滤: `service_name: "api-gateway"`
   - 按日志级别过滤: `level: "error"`
   - 按链路追踪ID过滤: `trace_id: "abcd1234"`

常用查询示例:
```
level: "error" AND service_name: "msg-server"
message: "*连接失败*" OR message: "*超时*"
user_id: "12345" AND level: "error"
```

### 链路追踪查询（Jaeger）

1. 访问 Jaeger UI: http://localhost:16686
2. 从服务下拉列表中选择要查看的服务
3. 设置时间范围和过滤条件（如操作名、标签等）
4. 查看和分析请求追踪链

提示:
- 使用 Tags 筛选特定请求: `error=true` 显示出错的请求
- 按 Duration 排序找出耗时长的请求
- 查看 Dependencies 分析服务依赖关系

### 性能指标（Grafana）

1. 访问 Grafana: http://localhost:3000
2. 登录凭据: admin / admin (首次登录后请修改密码)
3. 浏览预配置的仪表板查看系统和服务性能

## 故障排除

### 日志系统问题

**日志没有出现在 Kibana 中:**
1. 确认微服务日志格式设置为 `json`
2. 检查 Logstash 是否正在运行: `docker-compose ps logstash`
3. 检查 Logstash 日志: `docker-compose logs logstash`
4. 确认 Elasticsearch 状态: `curl http://localhost:9200/_cluster/health`

**Filebeat 无法收集日志:**
1. 检查 Filebeat 配置中的路径是否正确
2. 查看 Filebeat 日志: `docker-compose logs filebeat`
3. 确认 `chmod 644` 日志文件以便 Filebeat 读取

### 链路追踪问题

**链路追踪数据未显示在 Jaeger UI 中:**
1. 确认 `telemetry.enabled` 设置为 `true`
2. 验证 `telemetry.endpoint` 正确指向 Jaeger
3. 确认已启用 `telemetry` 功能标志: `enabled = true`
4. 检查 Jaeger 日志: `docker-compose logs jaeger`

**跨服务追踪不完整:**
1. 确保微服务间传递了正确的上下文头
2. 检查 gRPC/HTTP 客户端是否正确传播追踪头

## 扩展功能

### 添加自定义仪表板

可以在 Grafana 中添加自定义仪表板显示重要指标：
1. 登录 Grafana
2. 点击 "+" > Dashboard
3. 添加面板，选择 Prometheus 数据源
4. 配置面板显示关注的指标

### 设置告警

可以配置告警在出现问题时通知团队：

**Elasticsearch/Kibana 告警:**
1. 在 Kibana 中创建 Watcher
2. 设置触发条件和通知方式

**Prometheus/Grafana 告警:**
1. 在 Grafana 面板中添加告警规则
2. 配置通知渠道（如 Email, Slack, WebHook）

### 添加自定义指标

在应用代码中添加自定义指标：

```rust
use metrics::{counter, gauge, histogram};

// 计数器: 跟踪事件发生次数
counter!("app.request.total", 1, "endpoint" => "/api/users");

// 仪表盘: 跟踪数值变化
gauge!("app.active_connections", active_connections as f64);

// 直方图: 测量分布情况
histogram!("app.request.duration", duration_ms as f64);
```

## 最佳实践

1. **生产环境优化:**
   - 日志级别设置为 INFO
   - 链路追踪采样率设置为 0.1-0.3
   - 定期归档或清理旧数据

2. **监控策略:**
   - 设置关键指标的告警
   - 创建业务指标仪表板
   - 定期审查性能趋势

3. **排障流程:**
   - 发现问题 → 检查链路追踪 → 查看详细日志
   - 使用标签关联日志和链路追踪数据
   - 记录排障过程，不断改进监控系统 