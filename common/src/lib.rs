pub mod config;
pub mod error;
pub mod grpc_client;
pub mod message;
pub mod models;
pub mod proto;
pub mod service_registry;
pub mod types;
pub mod utils;

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;
