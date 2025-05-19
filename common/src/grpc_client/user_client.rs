use anyhow::Result;
use tonic::Request;

use crate::proto::user::user_service_client::UserServiceClient;
use crate::proto::user::{CreateUserRequest, GetUserByIdRequest, GetUserByUsernameRequest, UpdateUserRequest, UserResponse, ForgetPasswordRequest, RegisterRequest, VerifyPasswordRequest, VerifyPasswordResponse, SearchUsersRequest, SearchUsersResponse, UserConfigRequest, UserConfigResponse};

use crate::grpc_client::GrpcServiceClient;

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

    /// 验证用户密码
    pub async fn verify_password(&self, request: VerifyPasswordRequest) -> Result<VerifyPasswordResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let response = client.verify_password(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 搜索用户
    pub async fn search_users(&self, query: &str, page: i32, page_size: i32) -> Result<SearchUsersResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let request = Request::new(SearchUsersRequest {
            query: query.to_string(),
            page,
            page_size,
        });

        let response = client.search_users(request).await?;
        Ok(response.into_inner())
    }

    /// 用户账号密码注册
    pub async fn register_by_username(&self, request: RegisterRequest) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let response = client.register_by_username(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 用户手机号注册
    pub async fn register_by_phone(&self, request: RegisterRequest) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let response = client.register_by_phone(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 忘记密码
    pub async fn forget_password(&self, request: ForgetPasswordRequest) -> Result<UserResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);

        let response = client.forget_password(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    // 查询用户设置
    pub async fn get_user_config(&self, user_id: &str) -> Result<UserConfigResponse> {
        let channel = self.service_client.get_channel().await?;
        let mut client = UserServiceClient::new(channel);
        let request = Request::new(UserConfigRequest {
            user_id: user_id.to_string(),
            allow_phone_search: Option::from(0 as i32),
            allow_id_search: Option::from(0 as i32),
            auto_load_video: Option::from(0 as i32),
            auto_load_pic: Option::from(0 as i32),
            msg_read_flag: Option::from(0 as i32),
        });
        let response = client.get_user_config(request).await?;
        Ok(response.into_inner())
    }
}
