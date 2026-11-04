//! 契约层
//!
//! 这里只放「如果每个服务各存一份，就一定会漂移」的东西：
//!
//! - `proto`：gRPC 服务与消息定义（由 build.rs 从 `proto/*.proto` 生成）
//! - `types`：对生成类型做的领域补充（消息分类等）
//! - `error`：跨 gRPC 边界需要统一转换的错误类型
//!
//! 约束：本 crate 不依赖数据库驱动之外的任何 IO 库，
//! 不得引入消息队列客户端、HTTP 框架的业务代码、Consul 客户端等。

// 错误类型里 tonic::Status 体积较大，而本 crate 的函数几乎都返回它。
// 统一在此豁免，避免每个函数各写一遍。
#![allow(clippy::result_large_err)]

pub mod error;
pub mod proto;
pub mod types;

pub use error::{Error, Result};
