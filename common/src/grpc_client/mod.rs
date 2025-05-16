pub mod user_client;
pub mod friend_client;
pub mod group_client;
pub mod macros;

pub use user_client::UserServiceGrpcClient;
pub use friend_client::FriendServiceGrpcClient;
pub use group_client::GroupServiceGrpcClient;

mod base;

pub use base::{GrpcClientFactory, GrpcServiceClient};

// 后续可以继续添加其他服务客户端模块
// pub mod auth_client;
// pub use auth_client::AuthServiceGrpcClient;

// 自动生成的客户端模块
pub mod generated;
pub use generated::*;
