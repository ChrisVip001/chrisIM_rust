// 导出各个服务模块
pub mod user_service;
pub mod friend_service;
pub mod group_service;
pub mod common_service;
pub mod chat_service;
pub mod common;

// 重新导出所有服务，方便外部直接使用
pub use user_service::UserServiceHandler;
pub use friend_service::FriendServiceHandler;
pub use group_service::GroupServiceHandler;
pub use common_service::CommonServiceHandler;
pub use chat_service::ChatServiceHandler; 