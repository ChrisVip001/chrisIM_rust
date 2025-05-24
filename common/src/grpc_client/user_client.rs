use anyhow::Result;
use tonic::Request;

use crate::proto::user::user_service_client::UserServiceClient;
use crate::proto::user::{CreateUserRequest, GetUserByIdRequest, GetUserByUsernameRequest, UpdateUserRequest, UserResponse, ForgetPasswordRequest, RegisterRequest, VerifyPasswordRequest, VerifyPasswordResponse, SearchUsersRequest, SearchUsersResponse, UserConfigRequest, UserConfigResponse, PhoneVerificationRequest, PhoneVerificationResponse, VerifyPhoneCodeRequest, VerifyPhoneCodeResponse};
use crate::service_discovery::LbWithServiceDiscovery;

/// 用户服务gRPC客户端
#[derive(Clone)]
pub struct UserServiceGrpcClient {
    service_client: UserServiceClient<LbWithServiceDiscovery>,
}

impl UserServiceGrpcClient {
    /// 创建新的用户服务客户端
    pub fn new(service_client: UserServiceClient<LbWithServiceDiscovery>) -> Self {
        Self { service_client }
    }

    /// 获取用户
    pub async fn get_user(&mut self, user_id: &str) -> Result<UserResponse> {

        let request = Request::new(GetUserByIdRequest {
            user_id: user_id.to_string(),
        });

        let response = self.service_client.get_user_by_id(request).await?;
        Ok(response.into_inner())
    }

    /// 按用户名获取用户
    pub async fn get_user_by_username(&mut self, username: &str) -> Result<UserResponse> {

        let request = Request::new(GetUserByUsernameRequest {
            username: username.to_string(),
        });

        let response = self.service_client.get_user_by_username(request).await?;
        Ok(response.into_inner())
    }

    /// 创建用户
    pub async fn create_user(&mut self, request: CreateUserRequest) -> Result<UserResponse> {

        let response = self.service_client.create_user(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 更新用户
    pub async fn update_user(&mut self, request: UpdateUserRequest) -> Result<UserResponse> {

        let response = self.service_client.update_user(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 验证用户密码
    pub async fn verify_password(&mut self, request: VerifyPasswordRequest) -> Result<VerifyPasswordResponse> {

        let response = self.service_client.verify_password(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 搜索用户
    pub async fn search_users(&mut self, query: &str, page: i32, page_size: i32) -> Result<SearchUsersResponse> {

        let request = Request::new(SearchUsersRequest {
            query: query.to_string(),
            page,
            page_size,
        });

        let response = self.service_client.search_users(request).await?;
        Ok(response.into_inner())
    }

    /// 用户账号密码注册
    pub async fn register_by_username(&mut self, request: RegisterRequest) -> Result<UserResponse> {

        let response = self.service_client.register_by_username(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 用户手机号注册
    pub async fn register_by_phone(&mut self, request: RegisterRequest) -> Result<UserResponse> {

        let response = self.service_client.register_by_phone(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    /// 忘记密码
    pub async fn forget_password(&mut self, request: ForgetPasswordRequest) -> Result<UserResponse> {

        let response = self.service_client.forget_password(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    // 查询用户设置
    pub async fn get_user_config(&mut self, user_id: &str) -> Result<UserConfigResponse> {

        let request = Request::new(UserConfigRequest {
            user_id: user_id.to_string(),
            allow_phone_search: Option::from(0 as i32),
            allow_id_search: Option::from(0 as i32),
            auto_load_video: Option::from(0 as i32),
            auto_load_pic: Option::from(0 as i32),
            msg_read_flag: Option::from(0 as i32),
        });
        let response = self.service_client.get_user_config(request).await?;
        Ok(response.into_inner())
    }

    // 保存用户设置
    pub async fn save_user_config(&mut self, request: UserConfigRequest) -> Result<UserConfigResponse> {
        let response = self.service_client.save_user_config(request).await?;
        Ok(response.into_inner())
    }
    
    /// 发送手机验证码
    pub async fn send_phone_verification_code(&mut self, request: PhoneVerificationRequest) -> Result<PhoneVerificationResponse> {
        let response = self.service_client.send_phone_verification_code(Request::new(request)).await?;
        Ok(response.into_inner())
    }
    
    /// 验证手机验证码
    pub async fn verify_phone_code(&mut self, request: VerifyPhoneCodeRequest) -> Result<VerifyPhoneCodeResponse> {
        let response = self.service_client.verify_phone_code(Request::new(request)).await?;
        Ok(response.into_inner())
    }
}
