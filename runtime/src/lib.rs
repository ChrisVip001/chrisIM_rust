//! 运行时基础设施
//!
//! 提供与具体业务无关的适配能力：
//!
//! - `config` / `configs`：配置加载与配置结构
//! - `logging`：日志与 OpenTelemetry 链路追踪初始化
//! - `service_register_center`：向 Consul 注册/注销服务
//! - `service_discovery` / `grpc_client`：服务发现与 gRPC 连接工厂
//! - `grpc`：gRPC 拦截器
//! - `auth`：JWT 签发与校验
//! - `sms`：短信通道
//! - `snowflake` / `utils`：ID 生成与通用工具
//!
//! 约束：这里不得出现 friend / group / user / message 等域概念。
//! 域客户端（如 `FriendServiceGrpcClient`）应由使用方基于
//! `grpc_client::base::get_chan` 自行构造。
//!
//! TODO(1.3): `grpc_client/{friend,group,user}_client.rs`（共约 1k 行）
//! 是 api-gateway 专用的业务封装，目前仍留在这里，待迁移到
//! `api-gateway/src/proxy/grpc_clients/` 后本条约束才真正成立。

// 错误类型定义在 contract，其中 tonic::Status 体积较大。
// 基础设施层几乎每个函数都返回它，为错误装箱的收益低于改动成本。
#![allow(clippy::result_large_err)]

pub mod auth;
pub mod config;
pub mod configs;
pub mod grpc;
pub mod grpc_client;
pub mod logging;
pub mod service;
pub mod service_discovery;
pub mod service_register_center;
pub mod sms;
pub mod snowflake;
pub mod utils;
