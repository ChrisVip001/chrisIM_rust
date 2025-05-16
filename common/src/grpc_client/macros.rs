/// 生成gRPC服务客户端代码的宏
/// 
/// 用法示例:
/// ```
/// use common::grpc_client::macros::generate_grpc_client;
/// 
/// // 生成UserServiceGrpcClient
/// generate_grpc_client!(
///     name: UserServiceGrpcClient, 
///     service: "user-service",
///     proto_path: crate::proto::user,
///     client_type: user_service_client::UserServiceClient,
///     methods: [
///         get_user(GetUserByIdRequest) -> UserResponse,
///         get_user_by_username(GetUserByUsernameRequest) -> UserResponse,
///         create_user(CreateUserRequest) -> UserResponse,
///         update_user(UpdateUserRequest) -> UserResponse,
///     ]
/// );
/// ```
#[macro_export]
macro_rules! generate_grpc_client {
    (
        name: $client_name:ident,
        service: $service_name:expr,
        proto_path: $proto_path:path,
        client_type: $client_type:path,
        methods: [
            $( $method:ident($req_type:ty) -> $resp_type:ty ),* $(,)?
        ]
    ) => {
        use anyhow::Result;
        use tonic::Request;
        use $proto_path::*;
        use $crate::grpc_client::GrpcServiceClient;

        /// gRPC服务客户端
        #[derive(Clone)]
        pub struct $client_name {
            service_client: GrpcServiceClient,
        }

        impl $client_name {
            /// 创建新的客户端
            pub fn new(service_client: GrpcServiceClient) -> Self {
                Self { service_client }
            }

            /// 从环境变量创建客户端
            pub fn from_env() -> Self {
                let service_client = GrpcServiceClient::from_env($service_name);
                Self::new(service_client)
            }

            $(
                /// 调用服务方法
                pub async fn $method(&self, $( req: $req_type )*) -> Result<$resp_type> {
                    let channel = self.service_client.get_channel().await?;
                    let mut client = <$client_type>::new(channel);

                    $(
                        let request = Request::new(req);
                        let response = client.$method(request).await?;
                        Ok(response.into_inner())
                    )*
                }
            )*
        }
    };
}

/// 简化的gRPC服务客户端生成宏
/// 
/// 用法示例:
/// ```
/// use common::grpc_client::macros::simple_grpc_client;
/// 
/// // 生成UserServiceGrpcClient
/// simple_grpc_client!(
///     UserServiceGrpcClient, 
///     "user-service",
///     crate::proto::user,
///     user_service_client::UserServiceClient
/// );
/// ```
#[macro_export]
macro_rules! simple_grpc_client {
    (
        $client_name:ident,
        $service_name:expr,
        $proto_path:path,
        $client_type:path
    ) => {
        use anyhow::Result;
        use tonic::Request;
        use $proto_path::*;
        use $crate::grpc_client::GrpcServiceClient;

        /// gRPC服务客户端 - 简化版
        #[derive(Clone)]
        pub struct $client_name {
            service_client: GrpcServiceClient,
        }

        impl $client_name {
            /// 创建新的客户端
            pub fn new(service_client: GrpcServiceClient) -> Self {
                Self { service_client }
            }

            /// 从环境变量创建客户端
            pub fn from_env() -> Self {
                let service_client = GrpcServiceClient::from_env($service_name);
                Self::new(service_client)
            }

            /// 获取gRPC客户端
            pub async fn get_client(&self) -> Result<$client_type<tonic::transport::Channel>> {
                let channel = self.service_client.get_channel().await?;
                Ok(<$client_type>::new(channel))
            }
            
            /// 通用调用方法，需要手动指定请求和响应类型
            pub async fn call<R>(&self, request: impl tonic::IntoRequest<R::Request>, method_name: &str) -> Result<R::Response>
            where
                R: tonic::client::GrpcService<tonic::body::BoxBody>,
                R::Error: Into<tonic::Status>,
                R::ResponseBody: tonic::codegen::Body + Send + 'static,
                <R::ResponseBody as tonic::codegen::Body>::Error: Into<tonic::Error> + Send,
                R::Future: Send,
            {
                let mut client = self.get_client().await?;
                
                // 这里需要通过反射或者模式匹配来调用正确的方法
                // 实际上不可能在这个宏中实现真正的动态方法调用
                // 所以这个方法更多是一个占位符，提醒用户需要手动实现具体方法
                
                Err(anyhow::anyhow!("需要在客户端中手动实现具体方法: {}", method_name))
            }
        }
    };
} 