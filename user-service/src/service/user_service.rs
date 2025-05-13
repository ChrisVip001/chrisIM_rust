use common::Error;
use common::proto::user::{
    user_service_server::UserService,
    CreateUserRequest, UpdateUserRequest, GetUserByIdRequest, GetUserByUsernameRequest,
    VerifyPasswordRequest, VerifyPasswordResponse, SearchUsersRequest, SearchUsersResponse,
    UserResponse, User as ProtoUser
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{info, error, debug};
use crate::model::user::{CreateUserData, UpdateUserData};
use crate::repository::user_repository::UserRepository;

/// 用户服务实现
pub struct UserServiceImpl {
    repository: UserRepository,
}

impl UserServiceImpl {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: UserRepository::new(pool),
        }
    }
}

#[tonic::async_trait]
impl UserService for UserServiceImpl {
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
        debug!("更新用户请求，用户ID: {}", req.user_id);
        
        // 转换请求数据
        let update_data = UpdateUserData::from(req.clone());
        
        // 更新用户
        let user = match self.repository.update_user(&req.user_id, update_data).await {
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
        match self.repository.verify_user_password(&req.username, &req.password).await {
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
        let (users, total) = match self.repository.search_users(&req.query, page, page_size).await {
            Ok(result) => result,
            Err(err) => {
                error!("搜索用户失败: {}", err);
                return Err(err.into());
            }
        };
        
        // 转换为响应格式
        let users: Vec<ProtoUser> = users.into_iter().map(ProtoUser::from).collect();
        
        // 返回响应
        Ok(Response::new(SearchUsersResponse {
            users,
            total,
        }))
    }
} 