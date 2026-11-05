//! 面向下游 gRPC 服务的客户端封装
//!
//! 每个结构体持有一个 tonic 生成的客户端，负责组装请求并取出响应体，
//! 把「服务发现 + 建连 + 具体 RPC」收在一处，业务层只调用方法。

pub mod chat_client;
pub mod friend_client;
pub mod group_client;
pub mod user_client;

pub use chat_client::ChatServiceGrpcClient;
pub use friend_client::FriendServiceGrpcClient;
pub use group_client::GroupServiceGrpcClient;
pub use user_client::UserServiceGrpcClient;
