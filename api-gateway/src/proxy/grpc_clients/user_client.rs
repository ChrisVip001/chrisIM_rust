use anyhow::Result;
use tonic::Request;

use common::proto::user::user_service_client::UserServiceClient;
use common::proto::user::{
    CreateUserRequest, GetUserByIdRequest, GetUserByUsernameRequest, UpdateUserRequest, UserResponse,
};

use common::grpc_client::GrpcServiceClient;

/// 用户服务gRPC客户端
#[derive(Clone)]
pub struct UserServiceGrpcClient {
    service_client: GrpcServiceClient,
}

impl UserServiceGrpcClient {
    /// 创建新的用户服务客户端
    pub fn new(service_client: GrpcServiceClient) -> Self {
        Self { service_client }
    }

    /// 从环境变量创建客户端
    pub fn from_env() -> Self {
        let service_client = GrpcServiceClient::from_env("user-service");
        Self::new(service_client)
    }

    /// 获取用户
    pub async fn get_user(&self, user_id: &str) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let request = Request::new(GetUserByIdRequest {
            user_id: user_id.to_string(),
        });

        let response = client.get_user_by_id(request).await?;
        Ok(response.into_inner())
    }

    /// 按用户名获取用户
    pub async fn get_user_by_username(&self, username: &str) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let request = Request::new(GetUserByUsernameRequest {
            username: username.to_string(),
        });

        let response = client.get_user_by_username(request).await?;
        Ok(response.into_inner())
    }

    /// 创建用户
    pub async fn create_user(&self, request: CreateUserRequest) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let response = client.create_user(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 更新用户
    pub async fn update_user(&self, request: UpdateUserRequest) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let response = client.update_user(Request::new(request)).await?;
        Ok(response.into_inner())
    }
}
