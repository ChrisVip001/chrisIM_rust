pub mod user_client;
pub mod friend_client;
pub mod group_client;
pub mod base;
// 声明子模块
pub mod client_factory;

pub use user_client::UserServiceGrpcClient;
pub use friend_client::FriendServiceGrpcClient;
pub use group_client::GroupServiceGrpcClient;

