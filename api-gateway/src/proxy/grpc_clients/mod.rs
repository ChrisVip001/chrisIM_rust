//! 面向 relation-service 的 gRPC 客户端封装
//!
//! 这些结构把「服务发现 + 建连 + 具体 RPC 方法」包在一起，属于 api-gateway 的业务适配层，
//! 因此放在网关侧而不是共享的 runtime 里：它们复制给别的服务不会造成契约不一致，
//! 放在共享层只会让「改好友接口」牵动整个 workspace 重编译。

pub mod friend_client;
pub mod group_client;
pub mod user_client;

pub use friend_client::FriendServiceGrpcClient;
pub use group_client::GroupServiceGrpcClient;
pub use user_client::UserServiceGrpcClient;
