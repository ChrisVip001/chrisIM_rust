pub mod user_client;

pub use user_client::UserServiceGrpcClient;

mod base;
pub use base::{GrpcServiceClient, GrpcClientFactory};

// 后续可以继续添加其他服务客户端模块
// pub mod auth_client;
// pub use auth_client::AuthServiceGrpcClient;