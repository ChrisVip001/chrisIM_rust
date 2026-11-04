//! gRPC 连接与通道工厂
//!
//! 这里只提供「如何连到某个服务」的能力，不认识任何具体业务：
//! - `base::get_chan` / `get_rpc_client`：按服务名建立带负载均衡的通道
//! - `client_factory`：给通道套上统一的构造入口
//!
//! 业务侧的客户端封装（如好友服务的 RPC 方法集）由各服务自己持有。

pub mod base;
pub mod client_factory;
