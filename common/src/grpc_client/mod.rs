pub mod base;
pub mod user_client;
pub mod friend_client;
pub mod group_client;
pub mod chat_client;
pub mod client_factory;

pub use user_client::UserServiceGrpcClient;
pub use friend_client::FriendServiceGrpcClient;
pub use group_client::GroupServiceGrpcClient;
pub use chat_client::ChatServiceGrpcClient;

