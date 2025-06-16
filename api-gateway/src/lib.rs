// API网关核心模块
pub mod auth;
pub mod metrics;
pub mod middleware;
pub mod proxy;
pub mod router;

// 工具模块
pub mod api_utils;
pub mod circuit_breaker;
pub mod rate_limit;

// 重新导出一些常用的类型
pub use common::grpc_client::friend_client::FriendServiceGrpcClient;
pub use common::grpc_client::group_client::GroupServiceGrpcClient;
pub use common::grpc_client::user_client::UserServiceGrpcClient; 