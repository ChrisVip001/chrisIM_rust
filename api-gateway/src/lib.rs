// 导出模块
pub mod api_doc;
pub mod api_utils;
pub mod auth;
pub mod circuit_breaker;
pub mod metrics;
pub mod middleware;
pub mod proxy;
pub mod rate_limit;
pub mod router;

// 重新导出一些常用的类型
pub use common::grpc_client::friend_client::FriendServiceGrpcClient;
pub use common::grpc_client::group_client::GroupServiceGrpcClient;
pub use common::grpc_client::user_client::UserServiceGrpcClient; 