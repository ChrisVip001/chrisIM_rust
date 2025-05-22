use chrono::FixedOffset;
use crate::model::user::{CreateUserData, ForgetPasswordData, RegisterUserData, UpdateUserData};
use crate::repository::user_repository::UserRepository;
use common::proto::user::{user_service_server::UserService, CreateUserRequest, ForgetPasswordRequest, GetUserByIdRequest, GetUserByUsernameRequest, RegisterRequest, SearchUsersRequest, SearchUsersResponse, UpdateUserRequest, User as ProtoUser, UserConfig, UserConfigRequest, UserConfigResponse, UserResponse, VerifyPasswordRequest, VerifyPasswordResponse};
use common::Error;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};
use common::utils::validate_phone;
use crate::model::user_config::UserConfigData;
use crate::repository::user_config_repository::UserConfigRepository;

/// 用户服务实现
pub struct UserServiceImpl {
    repository: UserRepository,
    user_config_repository: UserConfigRepository,
}

impl UserServiceImpl {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: UserRepository::new(pool.clone()),
            user_config_repository: UserConfigRepository::new(pool.clone()),
        }
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
        let reg_data = RegisterUserData::from(req);
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
        let reg_data = RegisterUserData::from(req);

        // 手机号格式校验
        if !validate_phone(&reg_data.phone) {
            error!("手机号格式不正确: {}", reg_data.phone);
            return Err(Status::invalid_argument("手机号格式不正确"));
        }

        // 短信验证码校验 todo

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
        debug!("用户忘记密码修改密码，手机号||账号: {}||{}", req.username, req.username);
        // 转换请求数据
        let forget_data = ForgetPasswordData::from(req);
        // 短信验证码校验 todo

        // 创建用户
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
        let create_data = CreateUserData::from(req);

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

        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
        }))
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

        // 返回响应
        Ok(Response::new(UserResponse {
            user: Some(ProtoUser::from(user)),
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
        debug!("验证用户密码请求，用户名: {}", req.username);

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

    /// 搜索用户
    async fn search_users(
        &self,
        request: Request<SearchUsersRequest>,
    ) -> std::result::Result<Response<SearchUsersResponse>, Status> {
        let req = request.into_inner();
        debug!("搜索用户请求，关键词: {}", req.query);

        // 设置默认分页参数
        let page = if req.page <= 0 { 1 } else { req.page };
        let page_size = if req.page_size <= 0 || req.page_size > 100 {
            10
        } else {
            req.page_size
        };

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

        // 转换为响应格式
        let users: Vec<ProtoUser> = users.into_iter().map(ProtoUser::from).collect();

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

        let user_config = match self.user_config_repository.save_user_config(&save_data).await {
            Ok(user_config) => user_config,
            Err(err) => {
                error!("查询用户设置失败: {}", err);
                return Err(err.into());
            }
        };
        info!("查询用户设置成功 {}", req.user_id);
        let proto_user_config = UserConfig {
            user_id: user_config.user_id,
            allow_phone_search: user_config.allow_phone_search,
            allow_id_search: user_config.allow_id_search,
            auto_load_video: user_config.auto_load_video,
            auto_load_pic: user_config.auto_load_pic,
            msg_read_flag: user_config.msg_read_flag,
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
}
