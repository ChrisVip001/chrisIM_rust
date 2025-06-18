use std::io::Read;
use chrono::{FixedOffset, Utc};
use crate::model::user::{CreateUserData, ForgetPasswordData, RegisterUserData, UpdateUserData};
use crate::repository::user_repository::UserRepository;
use common::proto::user::{user_service_server::UserService, CreateUserRequest, ForgetPasswordRequest, GetUserByIdRequest, GetUserByUsernameRequest, RegisterRequest, SearchUsersRequest, SearchUsersResponse, UpdateUserRequest, User as ProtoUser, UserConfig, UserConfigRequest, UserConfigResponse, UserResponse, VerifyPasswordRequest, VerifyPasswordResponse, PhoneVerificationRequest, PhoneVerificationResponse, VerifyPhoneCodeRequest, VerifyPhoneCodeResponse, DeactivateUserRequest, DeactivateUserResponse, UpdatePhoneRequest, UpdatePhoneResponse, CaptchaImageRequest, CaptchaImageResponse, EnhancedUserResponse, FriendshipStatus, GetEnhancedUserByIdRequest, GetUsersByIdsRequest, GetUsersByIdsResponse};
use common::Error;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};
use common::utils::{generate_captcha_image, generate_captcha_text, save_image_code, validate_phone, verify_image_code};
use crate::model::user_config::UserConfigData;
use crate::repository::user_config_repository::UserConfigRepository;
use std::sync::Arc;
use redis::{Client as RedisClient, Commands};
use common::sms::SmsService;
use common::sms::tencent::TencentSmsService;
use common::config::ConfigLoader;
use common::sms::VerificationAction;
use std::str::FromStr;
use common::grpc_client::base::get_rpc_client;
use common::proto::friend::{CheckFriendshipRequest, IsBlockedRequest};
use common::proto::friend::friend_service_client::FriendServiceClient;
use common::proto::user::user_service_client::UserServiceClient;
use common::service_discovery::LbWithServiceDiscovery;
use base64::Engine;
use base64::engine::general_purpose;
use uuid::Uuid;

/// 图片验证码前缀
const IMAGE_CODE_PREFIX: &str = "image:verification:code";
/// 用户服务实现
pub struct UserServiceImpl {
    repository: UserRepository,
    user_config_repository: UserConfigRepository,
    sms_service: Arc<dyn SmsService>,
    redis_client: RedisClient,
    friend_service_client: FriendServiceClient<LbWithServiceDiscovery>
}

impl UserServiceImpl {
    pub async fn new(pool: PgPool) -> anyhow::Result<Self> {
        // 获取配置
        let config = ConfigLoader::get_global().expect("获取全局配置失败");
        
        // 创建Redis客户端
        let redis_url = config.redis.url();
        let redis_client = RedisClient::open(redis_url)
            .expect("创建Redis客户端失败");
            
        // 创建短信服务
        let sms_service = Arc::new(TencentSmsService::new(
            redis_client.clone(), 
            Arc::new(config.sms.clone())
        ));

        let config = ConfigLoader::get_global().expect("获取全局配置失败");
        let friend_service_client = get_rpc_client::<FriendServiceClient<LbWithServiceDiscovery>>(&*config, "friend".to_string()).await?;

        Ok(Self {
            repository: UserRepository::new(pool.clone()),
            user_config_repository: UserConfigRepository::new(pool.clone()),
            sms_service,
            redis_client,
            friend_service_client,
        })
    }
    
    /// 处理用户信息时根据用户配置决定是否显示手机号
    async fn process_user_phone_display(&self, mut user: crate::model::user::User) -> Result<crate::model::user::User, Status> {
        // 获取用户配置
        let user_config = match self.user_config_repository.get_user_config(&user.id).await {
            Ok(config) => config,
            Err(err) => {
                error!("获取用户配置失败: {}", err);
                return Ok(user); // 配置获取失败时，默认显示原始信息
            }
        };
        
        // 根据show_phone配置决定是否显示手机号
        if let Some(show_phone) = user_config.show_phone {
            if show_phone == 2 { // 2表示不显示手机号
                // 将手机号处理为脱敏状态
                if !user.phone.is_empty() {
                    // 保留前三位和后四位，中间用星号代替
                    if user.phone.len() >= 7 {
                        let prefix = &user.phone[0..3];
                        let suffix = &user.phone[user.phone.len() - 4..];
                        let stars = "*".repeat(user.phone.len() - 7);
                        user.phone = format!("{}{}{}", prefix, stars, suffix);
                    }
                }
            }
        }
        
        Ok(user)
    }
    
    /// 发送手机验证码
    async fn send_phone_verification_code(&self, phone: &str, action_str: &str) -> Result<String, Status> {
        // 检查手机号格式
        if !validate_phone(phone) {
            return Err(Status::invalid_argument("手机号格式不正确"));
        }
        
        // 解析验证码用途
        let action = match VerificationAction::from_str(action_str) {
            Ok(action) => action,
            Err(_) => {
                error!("未知的验证码用途: {}", action_str);
                return Err(Status::invalid_argument(format!("未知的验证码用途: {}", action_str)));
            }
        };
        
        // 根据验证码用途判断是否需要验证用户存在性
        match action {
            // 这些操作需要验证用户存在
            VerificationAction::Login | 
            VerificationAction::ResetPassword | 
            VerificationAction::BindPhone | 
            VerificationAction::Deactivate => {
                // 通过手机号检查用户是否存在
                match self.repository.get_user_by_phone(phone).await {
                    Ok(_) => {}, // 用户存在，继续处理
                    Err(err) => {
                        error!("手机号对应用户不存在: {}, 错误: {}", phone, err);
                        return Err(Status::not_found(format!("用户不存在: {}", phone)));
                    }
                }
            },
            // 注册操作不需要验证用户存在
            VerificationAction::Register |
            VerificationAction::ChangePhone => {
                // 注册时，反而应该确保用户不存在
                match self.repository.get_user_by_phone(phone).await {
                    Ok(_) => {
                        // 用户已存在，返回错误
                        error!("手机号已注册: {}", phone);
                        return Err(Status::already_exists(format!("新手机号已注册: {}", phone)));
                    },
                    Err(_) => {
                        // 用户不存在，可以发送注册验证码
                        debug!("手机号未注册，可以发送注册验证码: {}", phone);
                    }
                }
            }
        }
        
        // 添加国家代码前缀（假设都是中国号码）
        let phone_with_prefix = if phone.starts_with("+") {
            phone.to_string()
        } else {
            format!("+86{}", phone)
        };
        
        // 发送验证码
        match self.sms_service.send_verification_code(&phone_with_prefix, action).await {
            Ok(code) => {
                debug!("成功发送{}验证码到手机号: {}", action.as_str(), phone);
                Ok(code)
            },
            Err(err) => {
                error!("发送{}验证码失败: {}", action.as_str(), err);
                Err(Status::unavailable(format!("发送{}验证码失败: {}", action.as_str(), err)))
            }
        }
    }
    
    /// 验证手机验证码
    async fn verify_phone_code(&self, phone: &str, code: &str, action_str: &str) -> Result<bool, Status> {
        // 检查手机号格式
        if !validate_phone(phone) {
            return Err(Status::invalid_argument("手机号格式不正确"));
        }
        
        // 解析验证码用途
        let action = match VerificationAction::from_str(action_str) {
            Ok(action) => action,
            Err(_) => {
                error!("未知的验证码用途: {}", action_str);
                return Err(Status::invalid_argument(format!("未知的验证码用途: {}", action_str)));
            }
        };
        
        // 添加国家代码前缀（假设都是中国号码）
        let phone_with_prefix = if phone.starts_with("+") {
            phone.to_string()
        } else {
            format!("+86{}", phone)
        };
        
        // 验证码
        match self.sms_service.verify_code(&phone_with_prefix, code, action).await {
            Ok(is_valid) => {
                if is_valid {
                    debug!("{}验证码验证成功，手机号: {}", action.as_str(), phone);
                    Ok(true)
                } else {
                    debug!("{}验证码不匹配，手机号: {}", action.as_str(), phone);
                    Ok(false)
                }
            },
            Err(err) => {
                error!("验证{}验证码失败: {}", action.as_str(), err);
                Err(Status::internal(format!("验证{}验证码失败: {}", action.as_str(), err)))
            }
        }
    }

    /// 生成唯一的用户自定义ID
    ///
    /// 生成格式为 "myid-XXXXXXXX" 的唯一ID，如果生成的ID已存在，
    /// 会重新尝试生成，最多尝试5次，之后会使用时间戳确保唯一性
    async fn generate_unique_user_id(&self) -> String {
        let mut attempts = 0;
        const MAX_ATTEMPTS: i32 = 5; // 最大尝试次数，防止无限循环

        loop {
            // 使用工具函数生成ID
            let custom_id = common::utils::generate_user_custom_id();

            // 检查ID是否已存在
            match self.repository.is_custom_id_exists(&custom_id).await {
                // 数据库错误
                Err(err) => {
                    error!("检查自定义ID时发生错误: {}", err);
                    attempts += 1;
                },
                // ID已存在
                Ok(true) => {
                    attempts += 1;
                    debug!("自定义ID已存在，尝试生成新ID: {}", custom_id);
                },
                // ID不存在，可以使用
                Ok(false) => {
                    debug!("生成的自定义ID可用: {}", custom_id);
                    return custom_id;
                }
            }

            // 达到最大尝试次数后使用时间戳作为后缀，保证唯一性
            if attempts >= MAX_ATTEMPTS {
                let timestamp = Utc::now().timestamp_millis();
                let final_id = format!("myid-{}", timestamp);
                debug!("达到最大尝试次数，使用时间戳ID: {}", final_id);
                return final_id;
            }
        }
    }

    /// 检查用户在线状态
    async fn check_user_online_status(&self, user_id: &str) -> Result<bool, Status> {
        // TODO 从Redis检查用户是否在线
        let mut redis_conn = self.redis_client.get_connection().map_err(|e| {
            error!("获取Redis连接失败: {}", e);
            Status::internal("获取在线状态失败")
        })?;

        let is_online: bool = redis::cmd("SISMEMBER")
            .arg("online_users")
            .arg(user_id)
            .query(&mut redis_conn)
            .unwrap_or(false);

        Ok(is_online)
    }

    /// 检查用户好友和拉黑状态
    async fn check_friend_and_blacklist_status(&self, current_user_id: &str, target_user_id: &str) -> Result<(i32, bool), Status> {
        let check_friendship_request =  CheckFriendshipRequest {
            user_id: current_user_id.to_string(),
            friend_id: target_user_id.to_string(),
        };
        let friend_status = self.friend_service_client.clone().check_friendship(check_friendship_request).await?.into_inner();

        let check_block_request = IsBlockedRequest {
            user_id: current_user_id.to_string(),
            blocked_user_id: target_user_id.to_string(),
        };
        let is_blocked = self.friend_service_client.clone().is_blocked(check_block_request).await?.into_inner().is_blocked;
        Ok((friend_status.status, is_blocked))
    }
}

#[tonic::async_trait]
impl UserService for UserServiceImpl {

    /// 用户账号密码注册
    async fn register_by_username(
        &self,
        request: Request<RegisterRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        debug!("用户账号密码注册请求，用户名: {}", req.username);
        // 转换请求数据
        let mut reg_data = RegisterUserData::from(req.clone());
        
        // 生成用户自定义ID
        reg_data.custom_id = self.generate_unique_user_id().await;

        // 图片验证码校验
        if !verify_image_code(&req.image_code_key, &req.image_code) {
            error!("图片验证码错误: {}", req.image_code);
            return Err(Status::invalid_argument("图片验证码错误"));
        }

        // 创建用户
        let user = match self.repository.register_user(reg_data).await {
            Ok(user) => user,
            Err(err) => {
                error!("用户注册失败: {}", err);
                return Err(err.into());
            }
        };
        info!("注册用户成功 {}", user.username);

       
        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
        }))
    }

    /// 用户手机号注册
    async fn register_by_phone(
        &self,
        request: Request<RegisterRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        debug!("用户手机号注册，手机号: {}", req.phone);
        // 转换请求数据
        let mut reg_data = RegisterUserData::from(req.clone());

        
        // 生成用户自定义ID
        reg_data.custom_id = self.generate_unique_user_id().await;
        
        // 如果没有指定用户名，则使用自定义ID作为用户名
        if reg_data.username.is_empty() {
            reg_data.username = reg_data.custom_id.clone();
        }

        // 图片验证码校验
        if !verify_image_code(&req.image_code_key, &req.image_code) {
            error!("图片验证码错误: {}", req.image_code);
            return Err(Status::invalid_argument("图片验证码错误"));
        }

        // 手机号格式校验
        if !validate_phone(&reg_data.phone) {
            error!("手机号格式不正确: {}", reg_data.phone);
            return Err(Status::invalid_argument("手机号格式不正确"));
        }

        // 短信验证码校验
        if req.verify_code.is_empty() {
            return Err(Status::invalid_argument("验证码不能为空"));
        }

        // 注册时，应该确保用户不存在
        match self.repository.get_user_by_phone(&reg_data.phone).await {
            Ok(_) => {
                error!("手机号已注册: {}", reg_data.phone);
                return Err(Status::already_exists(format!("手机号已注册: {}", reg_data.phone)));
            },
            Err(_) => {
                debug!("手机号未注册，可以继续注册流程: {}", reg_data.phone);
            }
        }

        let verify_result = self.verify_phone_code(&reg_data.phone, &req.verify_code, "register").await?;
        if !verify_result {
            return Err(Status::invalid_argument("验证码错误"));
        }

        // 创建用户
        let user = match self.repository.register_user(reg_data).await {
            Ok(user) => user,
            Err(err) => {
                error!("用户注册失败: {}", err);
                return Err(err.into());
            }
        };
        info!("注册用户成功 {}", user.phone);

        
        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
        }))
    }

    /// 忘记密码
    async fn forget_password(
        &self,
        request: Request<ForgetPasswordRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        debug!("用户忘记密码修改密码，手机号: {}", req.phone);
        // 转换请求数据
        let forget_data = ForgetPasswordData::from(req.clone());
        
        // 短信验证码校验
        if !req.phone.is_empty() {
            if req.verify_code.is_empty() {
                return Err(Status::invalid_argument("验证码不能为空"));
            }
            
            let verify_result = self.verify_phone_code(&forget_data.phone, &req.verify_code, "reset_password").await?;
            if !verify_result {
                return Err(Status::invalid_argument("验证码错误"));
            }
        }

        // 修改密码
        let user = match self.repository.forget_password(forget_data).await {
            Ok(user) => user,
            Err(err) => {
                error!("修改密码失败: {}", err);
                return Err(err.into());
            }
        };
        info!("修改密码成功 {}", user.phone);

      
        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
        }))
    }

    /// 创建用户
    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        debug!("创建用户请求，用户名: {}", req.username);

        // 转换请求数据
        let mut create_data = CreateUserData::from(req);
        
        // 生成用户自定义ID
        create_data.custom_id = self.generate_unique_user_id().await;
        // 创建用户
        let user = match self.repository.create_user(create_data).await {
            Ok(user) => user,
            Err(err) => {
                error!("创建用户失败: {}", err);
                return Err(err.into());
            }
        };

        info!("成功创建用户 {}", user.id);

       
        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
        }))
    }

    /// 通过ID获取用户
    async fn get_user_by_id(
        &self,
        request: Request<GetUserByIdRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        debug!("通过ID获取用户请求，ID: {}", req.user_id);

        // 查询用户
        let user = match self.repository.get_user_by_id(&req.user_id).await {
            Ok(user) => user,
            Err(err) => {
                error!("通过ID获取用户失败: {}", err);
                return Err(err.into());
            }
        };

        let mut processed_user = user.clone();
        // 处理用户信息时根据用户配置决定是否显示手机号
        if &req.current_user_id != &user.id {
            processed_user = self.process_user_phone_display(user).await?;
        }
        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(processed_user)),
        }))
    }

    /// 增强的通过ID获取用户（包含好友状态、拉黑状态、在线状态）
    async fn get_enhanced_user_by_id(
        &self,
        request: Request<GetEnhancedUserByIdRequest>,
    ) -> std::result::Result<Response<EnhancedUserResponse>, Status> {
        let req = request.into_inner();
        debug!("增强的通过ID获取用户请求，当前用户: {}, 目标用户: {}", req.current_user_id, req.user_id);

        // 查询用户基本信息
        let user = match self.repository.get_user_by_id(&req.user_id).await {
            Ok(user) => user,
            Err(err) => {
                error!("通过ID获取用户失败: {}", err);
                return Err(err.into());
            }
        };

        let mut processed_user = user.clone();
        // 处理用户信息时根据用户配置决定是否显示手机号
        if &req.current_user_id != &user.id {
            processed_user = self.process_user_phone_display(user).await?;
        }

        // 检查在线状态
        let is_online = self.check_user_online_status(&req.user_id).await.unwrap_or(false);

        // 检查好友关系和拉黑状态
        let (friend_status, is_blocked) = self.check_friend_and_blacklist_status(&req.current_user_id, &req.user_id).await?;

        // 如果是好友关系，查找对应的好友关系列表
        if friend_status == 1 {
            let friend_relation_request = Request::new(common::proto::friend::GetFriendRelationRequest {
                user_id: req.current_user_id.clone(),
                friend_id: req.user_id.clone(),
            });
            
            let friend_relation_resp = self.friend_service_client.clone().get_friend_relation(friend_relation_request).await?.into_inner();
            
            // 获取好友所在分组列表
            let friend_groups_request = Request::new(common::proto::friend::GetFriendInGroupsRequest {
                user_id: req.current_user_id.clone(),
                friend_id: req.user_id.clone(),
            });
            
            let friend_groups_resp = match self.friend_service_client.clone().get_friend_in_groups(friend_groups_request).await {
                Ok(resp) => resp.into_inner(),
                Err(e) => {
                    // 日志记录错误，但不影响整体返回
                    tracing::error!("获取好友分组列表失败: {}", e);
                    common::proto::friend::GetFriendInGroupsResponse { groups: vec![] }
                }
            };
            
            // 转换分组信息
            let group_infos = friend_groups_resp.groups.into_iter()
                .map(|g| common::proto::user::FriendGroupInfo {
                    id: g.id,
                    group_name: g.group_name,
                })
                .collect();
            
            // 返回增强的响应
            Ok(Response::new(EnhancedUserResponse {
                user: Some(ProtoUser::from(processed_user)),
                is_blocked,
                friend_status: friend_status as i32,
                is_online,
                friend_relation: friend_relation_resp.friend.map(|f| common::proto::user::FriendRelation {
                    remark: f.remark,
                    is_starred: f.is_starred,
                    is_top: f.is_top,
                    friend_type: f.friend_type,
                    groups: group_infos,
                }),
            }))
        } else {
            // 返回增强的响应，但没有好友关系信息
            Ok(Response::new(EnhancedUserResponse {
                user: Some(ProtoUser::from(processed_user)),
                is_blocked,
                friend_status: friend_status as i32,
                is_online,
                friend_relation: None,
            }))
        }
    }

    /// 通过用户名获取用户
    async fn get_user_by_username(
        &self,
        request: Request<GetUserByUsernameRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        debug!("通过用户名获取用户请求，用户名: {}", req.username);

        // 查询用户
        let user = match self.repository.get_user_by_username(&req.username).await {
            Ok(user) => user,
            Err(err) => {
                error!("通过用户名获取用户失败: {}", err);
                return Err(err.into());
            }
        };

        // 处理用户信息时根据用户配置决定是否显示手机号
        let processed_user = self.process_user_phone_display(user).await?;

        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(processed_user)),
        }))
    }

    /// 更新用户
    async fn update_user(
        &self,
        request: Request<UpdateUserRequest>,
    ) -> std::result::Result<Response<UserResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id.clone().unwrap_or_default();
        debug!("更新用户请求，用户ID: {}", user_id);

        // 转换请求数据
        let update_data = UpdateUserData::from(req.clone());

        // 更新用户
        let user = match self.repository.update_user(&user_id, update_data).await {
            Ok(user) => user,
            Err(err) => {
                error!("更新用户失败: {}", err);
                return Err(err.into());
            }
        };

        info!("成功更新用户 {}", user.id);

        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
        }))
    }

    /// 验证用户密码
    async fn verify_password(
        &self,
        request: Request<VerifyPasswordRequest>,
    ) -> std::result::Result<Response<VerifyPasswordResponse>, Status> {
        let req = request.into_inner();
        debug!("验证用户密码请求，用户名/手机号: {}", req.username);

        // 验证密码
        match self
            .repository
            .verify_user_password(&req.username, &req.password)
            .await
        {
            Ok(user) => {
                debug!("密码验证成功，用户ID: {}", user.id);
                
                // 返回响应
                Ok(Response::new(VerifyPasswordResponse {
                    valid: true,
                    user: Some(ProtoUser::from(user)),
                }))
            }
            Err(err) => {
                // 如果是认证错误（密码不匹配），返回valid=false
                if let Error::Authentication(_) = err {
                    debug!("密码验证失败，用户名: {}", req.username);
                    return Ok(Response::new(VerifyPasswordResponse {
                        valid: false,
                        user: None,
                    }));
                }

                // 其他错误（如用户不存在等）
                error!("验证密码过程中发生错误: {}", err);
                Err(err.into())
            }
        }
    }

    // 验证手机号登录验证码（用于登录）
    async fn verify_phone_code_login(&self, request: Request<VerifyPhoneCodeRequest>) -> Result<Response<VerifyPasswordResponse>, Status> {
        let req = request.into_inner();
        debug!("验证用户验证码登录，手机号: {}", req.phone);

        // 验证手机号
        if !validate_phone(&req.phone) {
            return Err(Status::invalid_argument("手机号格式不正确"));
        }

        // 验证验证码
        if req.code.is_empty() {
            return Err(Status::invalid_argument("验证码不能为空"));
        }
        
        let verify_result = self.verify_phone_code(&req.phone, &req.code, "login").await?;
        if !verify_result {
            return Err(Status::invalid_argument("验证码错误"));
        }
        
        // 通过手机号获取用户
        let user = match self.repository.get_user_by_phone(&req.phone).await {
            Ok(user) => user,
            Err(err) => {
                error!("通过手机号获取用户失败: {}", err);
                return Err(err.into());
            }
        };
        
        debug!("手机验证码登录成功，用户ID: {}", user.id);
        
        // 返回响应
        Ok(Response::new(VerifyPasswordResponse {
            valid: true,
            user: Some(ProtoUser::from(user)),
        }))
    }

    /// 搜索用户
    async fn search_users(
        &self,
        request: Request<SearchUsersRequest>,
    ) -> std::result::Result<Response<SearchUsersResponse>, Status> {
        let req = request.into_inner();
        debug!("搜索用户请求，关键词: {}", req.query);

        // 设置默认分页参数
        let page = if req.page <= 0 { 1 } else { req.page };
        let page_size = if req.page_size <= 0 { 10 } else { req.page_size };

        // 搜索用户
        let (users, total) = match self
            .repository
            .search_users(&req.query, page, page_size)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                error!("搜索用户失败: {}", err);
                return Err(err.into());
            }
        };

        // 处理每个用户的手机号显示
        let mut processed_users = Vec::with_capacity(users.len());
        for user in users {
            let processed_user = match self.process_user_phone_display(user).await {
                Ok(user) => user,
                Err(_) => continue, // 处理失败时跳过该用户
            };
            processed_users.push(processed_user);
        }

        // 转换为响应格式
        let users: Vec<ProtoUser> = processed_users.into_iter().map(ProtoUser::from).collect();

        // 返回响应
        Ok(Response::new(SearchUsersResponse { users, total }))
    }

    /******************************用户设置*************************************/
    /// 查询用户设置
    async fn get_user_config(
        &self,
        request: Request<UserConfigRequest>,
    ) -> std::result::Result<Response<UserConfigResponse>, Status> {
        let req = request.into_inner();
        debug!("查询用户设置请求，id: {}", req.user_id);
        let user_config = match self.user_config_repository.get_user_config(&req.user_id).await {
            Ok(user_config) => user_config,
            Err(err) => {
                error!("查询用户设置失败: {}", err);
                return Err(err.into());
            }
        };
        let proto_user_config = UserConfig {
            user_id: user_config.user_id,
            allow_phone_search: user_config.allow_phone_search,
            allow_id_search: user_config.allow_id_search,
            auto_load_video: user_config.auto_load_video,
            auto_load_pic: user_config.auto_load_pic,
            msg_read_flag: user_config.msg_read_flag,
            sound_enabled: user_config.sound_enabled,
            vibration_enabled: user_config.vibration_enabled,
            show_phone: user_config.show_phone,
            create_time: user_config.create_time.map(|dt| prost_types::Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            update_time: user_config.update_time.map(|dt| prost_types::Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
        };

        // 返回响应
        Ok(Response::new(UserConfigResponse {
            user_config: Some(UserConfig::from(proto_user_config)),
        }))
    }

    /// 保存用户设置
    async fn save_user_config(
        &self,
        request: Request<UserConfigRequest>,
    ) -> std::result::Result<Response<UserConfigResponse>, Status> {
        let req = request.into_inner();
        debug!("保存用户设置请求，id: {}", req.user_id);



        // 转换请求数据
        let save_data = UserConfigData::from(req.clone());

        // 获取旧的配置，用于比较变更
        let old_config = match self.user_config_repository.get_user_config(&req.user_id).await {
            Ok(config) => Some(config),
            Err(_) => None,
        };

        // 保存用户配置
        let user_config = match self.user_config_repository.save_user_config(&save_data).await {
            Ok(user_config) => user_config,
            Err(err) => {
                error!("保存用户设置失败: {}", err);
                return Err(err.into());
            }
        };



        info!("保存用户设置成功 {}", req.user_id);
        let proto_user_config = UserConfig {
            user_id: user_config.user_id,
            allow_phone_search: user_config.allow_phone_search,
            allow_id_search: user_config.allow_id_search,
            auto_load_video: user_config.auto_load_video,
            auto_load_pic: user_config.auto_load_pic,
            msg_read_flag: user_config.msg_read_flag,
            sound_enabled: user_config.sound_enabled,
            vibration_enabled: user_config.vibration_enabled,
            show_phone: user_config.show_phone,
            create_time: user_config.create_time.map(|dt| prost_types::Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            update_time: user_config.update_time.map(|dt| prost_types::Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
        };

        // 返回响应
        Ok(Response::new(UserConfigResponse {
            user_config: Some(UserConfig::from(proto_user_config)),
        }))
    }

    /// 发送手机验证码
    async fn send_phone_verification_code(
        &self,
        request: Request<PhoneVerificationRequest>,
    ) -> std::result::Result<Response<PhoneVerificationResponse>, Status> {
        let req = request.into_inner();
        debug!("发送手机验证码请求，手机号: {}, 操作类型: {}", req.phone, req.action);
        
        match self.send_phone_verification_code(&req.phone, &req.action).await {
            Ok(_) => {
                // 成功发送验证码
                Ok(Response::new(PhoneVerificationResponse {
                    success: true,
                    message: "验证码已发送".to_string(),
                }))
            },
            Err(err) => {
                // 发送验证码失败
                Ok(Response::new(PhoneVerificationResponse {
                    success: false,
                    message: err.to_string(),
                }))
            }
        }
    }
    
    /// 验证手机验证码
    async fn verify_phone_code(
        &self,
        request: Request<VerifyPhoneCodeRequest>,
    ) -> std::result::Result<Response<VerifyPhoneCodeResponse>, Status> {
        let req = request.into_inner();
        debug!("验证手机验证码请求，手机号: {}, 操作类型: {}", req.phone, req.action);
        
        match self.verify_phone_code(&req.phone, &req.code, &req.action).await {
            Ok(is_valid) => {
                Ok(Response::new(VerifyPhoneCodeResponse {
                    valid: is_valid,
                    message: if is_valid { 
                        "验证码验证成功".to_string() 
                    } else { 
                        "验证码错误".to_string()
                    },
                }))
            },
            Err(err) => {
                Ok(Response::new(VerifyPhoneCodeResponse {
                    valid: false,
                    message: err.to_string(),
                }))
            }
        }
    }

    /// 注销用户账号
    async fn deactivate_user(
        &self,
        request: Request<DeactivateUserRequest>,
    ) -> std::result::Result<Response<DeactivateUserResponse>, Status> {
        let request = request.into_inner();
        
        info!("用户注销请求: user_id={}, phone={}", request.user_id, request.phone);
        
        // 手机号格式校验
        if !validate_phone(&request.phone) {
            error!("手机号格式不正确: {}", request.phone);
            return Err(Status::invalid_argument("手机号格式不正确"));
        }
        
        // 短信验证码校验
        if request.verify_code.is_empty() {
            return Err(Status::invalid_argument("验证码不能为空"));
        }

        // 确认手机号与用户匹配
        match self.repository.get_user_by_id(&request.user_id).await {
            Ok(user) => {
                if user.phone != request.phone {
                    error!("提供的手机号与用户绑定的手机号不匹配");
                    return Err(Status::permission_denied("提供的手机号与用户绑定的手机号不匹配"));
                }
            },
            Err(err) => {
                error!("获取用户信息失败: {}", err);
                return Err(err.into());
            }
        }


        // 验证码验证
        match self.verify_phone_code(&request.phone, &request.verify_code, "deactivate").await {
            Ok(is_valid) => {
                if !is_valid {
                    return Err(Status::invalid_argument("验证码错误"));
                }
            },
            Err(err) => {
                error!("验证码验证失败: {}", err);
                return Err(err);
            }
        }
        
      
        // 执行注销操作
        match self.repository.deactivate_user(&request.user_id).await {
            Ok(success) => {
                if success {
                    info!("用户注销成功: {}", request.user_id);
                    Ok(Response::new(DeactivateUserResponse {
                        success: true,
                        message: "用户账号已成功注销".to_string(),
                    }))
                } else {
                    error!("用户注销失败，未找到用户: {}", request.user_id);
                    Ok(Response::new(DeactivateUserResponse {
                        success: false,
                        message: "用户注销失败，未找到用户".to_string(),
                    }))
                }
            },
            Err(err) => {
                error!("用户注销失败: {}", err);
                Err(Status::internal(format!("用户注销失败: {}", err)))
            }
        }
    }


    /// 修改手机号
    async fn update_phone(
        &self,
        request: Request<UpdatePhoneRequest>,
    ) -> std::result::Result<Response<UpdatePhoneResponse>, Status> {
        let req = request.into_inner();
        debug!("用户修改手机号请求，用户ID: {}", req.user_id);

        // 验证用户ID
        let user = match self.repository.get_user_by_id(&req.user_id).await {
            Ok(user) => user,
            Err(err) => {
                error!("获取用户信息失败: {}", err);
                return Err(Status::not_found("用户不存在"));
            }
        };

        // 验证用户密码
        let password_valid = match self.repository.verify_user_password_by_id(&user.id, &req.password).await {
            Ok(valid) => valid,
            Err(err) => {
                error!("验证密码失败: {}", err);
                return Err(err.into());
            }
        };

        if !password_valid {
            return Err(Status::invalid_argument("密码错误"));
        }

        // 验证新手机号的格式
        if !validate_phone(&req.new_phone) {
            return Err(Status::invalid_argument("新手机号格式不正确"));
        }

        // 检查新手机号是否已被其他用户使用
        if let Ok(existing_user) = self.repository.get_user_by_phone(&req.new_phone).await {
            if existing_user.id != req.user_id {
                return Err(Status::already_exists("该手机号已被其他用户使用"));
            }
        }

        // 验证验证码
        if req.verify_code.is_empty() {
            return Err(Status::invalid_argument("验证码不能为空"));
        }

        let verify_result = self.verify_phone_code(&req.new_phone, &req.verify_code, "change_phone").await?;
        if !verify_result {
            return Err(Status::invalid_argument("验证码错误"));
        }
        
        

        // 更新用户手机号
        match self.repository.update_phone(&req.user_id, &req.new_phone).await {
            Ok(updated_user) => {
                info!("用户手机号更新成功，用户ID: {}", req.user_id);

                Ok(Response::new(UpdatePhoneResponse {
                    success: true,
                    message: "手机号更新成功".to_string(),
                    user: Some(ProtoUser::from(user)),
                }))
            }
            Err(err) => {
                error!("更新手机号失败: {}", err);
                Err(err.into())
            }
        }
    }

    /// 图片验证码生成
    async fn generate_captcha_image(
        &self, request: Request<CaptchaImageRequest>
    ) -> std::result::Result<Response<CaptchaImageResponse>, Status> {
        let req = request.into_inner();
        debug!("图片验证码生成请求: 宽度:{},高度:{}", req.width, req.height);

        let captcha_text = generate_captcha_text();
        let captcha_image = generate_captcha_image(
            &(req.width as u32),
            &(req.height as u32),
            &captcha_text,
            &req.font_size
        );

        // 将图片转换为 Base64
        let base64_image = general_purpose::STANDARD.encode(&captcha_image);

        // 生成唯一 key 并存储到 Redis
        let code_str = Uuid::new_v4().simple();
        let code_key = format!("{}:{}", IMAGE_CODE_PREFIX, code_str);
        info!("图片验证码redisKey:{}", code_key);
        // 存储验证码到redis,有效时间5分钟
        save_image_code(&code_key.to_string(), &captcha_text,&300);
        Ok(Response::new(CaptchaImageResponse {
            success: true,
            image_content: base64_image,
            code_key: code_str.to_string(),
        }))
    }

    /// 根据用户ID列表获取用户列表
    async fn get_users_by_ids(
        &self,
        request: Request<GetUsersByIdsRequest>,
    ) -> std::result::Result<Response<GetUsersByIdsResponse>, Status> {
        let req = request.into_inner();
        debug!("根据ID列表获取用户请求，ID列表长度: {}", req.user_ids.len());

        // 获取用户列表
        let users = match self.repository.get_users_by_ids(&req.user_ids).await {
            Ok(users) => users,
            Err(err) => {
                error!("根据ID列表获取用户失败: {}", err);
                return Err(err.into());
            }
        };

        // 处理每个用户的手机号显示
        let mut processed_users = Vec::with_capacity(users.len());
        for user in users {
            let processed_user = match self.process_user_phone_display(user).await {
                Ok(user) => user,
                Err(_) => continue, // 处理失败时跳过该用户
            };
            processed_users.push(processed_user);
        }

        // 转换为响应格式
        let proto_users: Vec<ProtoUser> = processed_users.into_iter().map(ProtoUser::from).collect();

        // 返回响应
        Ok(Response::new(GetUsersByIdsResponse { 
            users: proto_users 
        }))
    }

}
