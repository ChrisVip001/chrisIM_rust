# API网关服务 (Gateway Service)

API网关服务是一个高性能、高可用的API网关，为IM系统的微服务架构提供统一入口。它支持动态路由、负载均衡、服务发现、认证授权、限流熔断、监控追踪等关键功能。

## 功能特性

### 1. 路由转发
- **动态路由配置**：支持基于YAML文件的路由规则配置，包括路径前缀匹配、服务类型映射
- **路径重写**：支持路径前缀替换和正则表达式重写
- **HTTP/gRPC协议支持**：支持HTTP REST API和gRPC服务的统一网关入口

### 2. 认证与授权
- **多种认证方式**：
  - JWT令牌认证
  - API Key认证
  - OAuth2认证（预留）
- **权限控制**：
  - 基于角色的访问控制
  - 路径级别的权限控制
  - IP白名单配置
  - 路径白名单配置

### 3. 限流控制
- **多层级限流**：
  - 全局限流
  - 路径级别限流
  - API Key限流
  - IP地址限流
- **令牌桶算法**：高性能的令牌桶限流算法实现

### 4. 熔断降级
- **断路器模式**：自动检测服务健康状态，及时熔断不健康的服务
- **服务降级**：支持定制服务降级策略
- **自动恢复**：支持半开状态自动探测和恢复

### 5. 服务发现与负载均衡
- **Consul集成**：自动发现注册在Consul的服务实例
- **动态服务刷新**：定期刷新服务列表，保持最新的服务信息
- **负载均衡策略**：支持轮询、权重等负载均衡策略

### 6. 监控与指标
- **Prometheus集成**：提供丰富的监控指标
- **自定义指标**：请求计数、延迟、错误率等关键指标
- **健康检查端点**：提供服务健康状态检查

### 7. 链路追踪
- **OpenTelemetry集成**：支持分布式链路追踪
- **Jaeger支持**：可视化追踪链路
- **上下文传递**：跨服务传递追踪上下文

### 8. 动态配置
- **配置热加载**：支持配置文件的热加载和实时更新
- **配置监听**：基于文件系统监听的配置变更检测

## 快速开始

### 安装依赖
确保您已安装以下软件：
- Rust (1.75.0+)
- Docker (可选，用于容器化部署)
- Consul (用于服务发现)
- Jaeger (用于链路追踪)
- Prometheus (用于监控指标)

### 配置文件
网关服务使用YAML格式的配置文件。默认配置文件位于`config/gateway.yaml`。以下是配置文件的主要部分：

```yaml
# 路由配置
routes:
  routes:
    - id: "auth-service"
      name: "认证服务"
      path_prefix: "/api/auth"
      service_type: "Auth"
      require_auth: false
      # 更多配置...

# 限流配置
rate_limit:
  global:
    requests_per_second: 1000
    burst_size: 50
    enabled: true
  # 更多配置...

# 认证配置
auth:
  jwt:
    enabled: true
    secret: "your-secret-key"
    # 更多配置...
  # 更多认证方式...

# 其他配置...
```

### 启动服务

#### 编译
```bash
cargo build --release
```

#### 运行
```bash
./target/release/api-gateway -c config/gateway.yaml
```

### 命令行参数
- `-c, --config-file <FILE>`: 指定配置文件路径，默认为`config/gateway.yaml`
- `-h, --host <HOST>`: 指定监听地址，默认为环境变量`GATEWAY_HOST`或`127.0.0.1`
- `-p, --port <PORT>`: 指定监听端口，默认为环境变量`GATEWAY_PORT`或`8000`
- `--config <ENV_FILE>`: 指定环境变量文件，默认为`.env`

## 配置详解

### 路由配置
路由配置决定了API网关如何将请求转发到后端服务。

```yaml
routes:
  routes:
    - id: "service-id"              # 服务唯一标识
      name: "服务名称"               # 服务名称描述
      path_prefix: "/api/path"      # 路径前缀
      service_type: "ServiceType"   # 服务类型
      require_auth: true            # 是否需要认证
      methods: ["GET", "POST"]      # 允许的HTTP方法（空数组表示允许所有方法）
      rewrite_headers: {}           # 请求头重写规则
      path_rewrite:                 # 路径重写规则
        replace_prefix: "/"         # 前缀替换
```

### 限流配置
限流配置控制API请求的频率，防止过载和滥用。

```yaml
rate_limit:
  # 全局限流
  global:
    requests_per_second: 1000       # 每秒请求数
    burst_size: 50                  # 突发请求数
    enabled: true                   # 是否启用
  
  # 按路径限流
  path_rules:
    - path_prefix: "/api/auth/login"
      rule:
        requests_per_second: 5
        burst_size: 3
        enabled: true
```

### 认证配置
认证配置定义了API网关的安全策略。

```yaml
auth:
  # JWT配置
  jwt:
    enabled: true
    secret: "your-secret-key"
    issuer: "api-gateway"
    expiry_seconds: 86400           # 24小时
    header_name: "Authorization"
    header_prefix: "Bearer "
  
  # IP白名单
  ip_whitelist:
    - "127.0.0.1"
    - "::1"
  
  # 路径白名单（不需要认证）
  path_whitelist:
    - "/api/health"
    - "/api/auth/login"
```

### 熔断配置
熔断配置控制服务熔断和降级策略。

```yaml
circuit_breaker:
  enabled: true
  failure_threshold: 5              # 触发熔断的失败阈值
  half_open_timeout_secs: 30        # 半开状态超时时间
```

### 链路追踪配置
链路追踪配置定义了分布式追踪的行为。

```yaml
tracing:
  enable_opentelemetry: true
  jaeger_endpoint: "http://localhost:14268/api/traces"
  sampling_ratio: 0.1               # 采样率，1.0表示全采样
```

## API端点

### 健康检查
```
GET /health
```
返回网关服务的健康状态。

### 指标导出
```
GET /metrics
```
返回Prometheus格式的监控指标。

### 服务路由
所有配置的服务路由都可以通过对应的路径前缀访问。例如：

- 认证服务: `/api/auth/*`
- 用户服务: `/api/users/*`
- 好友服务: `/api/friends/*`
- 群组服务: `/api/groups/*`

## 扩展开发

### 添加新的路由
1. 在配置文件的`routes.routes`数组中添加新的路由规则
2. 重启服务或等待配置热加载生效

### 添加新的认证方式
1. 在`auth`模块中实现新的认证处理函数
2. 在统一认证入口中集成新的认证方式
3. 在配置文件中添加相应的配置项

### 集成新的服务类型
1. 在`config/routes_config.rs`的`ServiceType`枚举中添加新的服务类型
2. 在`proxy/service_proxy.rs`的`get_service_name`方法中添加新服务类型的映射

## 部署

### Docker部署
```bash
# 构建镜像
docker build -t api-gateway .

# 运行容器
docker run -p 8000:8000 -v $(pwd)/config:/app/config api-gateway
```

### Kubernetes部署
提供了Kubernetes部署的示例配置文件，位于`deploy/k8s`目录。

## 监控与维护

### 监控指标
主要监控指标包括：
- `gateway.requests.total`: 请求总数
- `gateway.request.duration`: 请求处理时间
- `gateway.responses.total`: 响应总数（按状态码）
- `gateway.errors.total`: 错误总数

### 日志级别
可以通过设置环境变量`RUST_LOG`来控制日志级别：
```bash
RUST_LOG=info,gateway_service=debug ./target/release/api-gateway
```

## 性能优化

API网关服务经过多项性能优化：
- 使用异步I/O和Tokio运行时
- 连接池和复用
- 高效的路由匹配算法
- 内存缓存
- 可调整的超时和重试策略

## 常见问题

### Q: 如何更新配置而不重启服务？
A: 修改配置文件后保存即可，网关服务会自动检测文件变化并重新加载。

### Q: 如何查看请求的追踪信息？
A: 访问Jaeger UI（默认为http://localhost:16686）查看追踪信息。

### Q: 如何增加新的限流规则？
A: 在配置文件的`rate_limit.path_rules`部分添加新的规则，保存后即可生效。

## 贡献指南

我们欢迎对API网关服务的贡献！如果您想贡献代码，请按照以下步骤操作：

1. Fork仓库
2. 创建特性分支
3. 提交更改
4. 推送到分支
5. 创建Pull Request

## 许可证

本项目采用MIT许可证。详情请参阅LICENSE文件。 