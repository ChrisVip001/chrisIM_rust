pub mod config;
pub mod error;
pub mod models;
pub mod proto;
pub mod utils;
pub mod service_registry;
pub mod message;
pub mod types;
pub mod grpc_client;

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>; 