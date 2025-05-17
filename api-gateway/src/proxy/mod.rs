pub mod grpc_client;
pub mod http_client;
pub mod service_proxy;
pub mod utils;
pub mod services;

// 导出公共接口
pub use grpc_client::GrpcClientFactoryImpl;
pub use grpc_client::GrpcClientFactory;
pub use service_proxy::ServiceProxy;
