use common::{config::AppConfig, Result, utils};
use common::proto::auth::{
    auth_service_server::AuthService,
    CreateTokenRequest, CreateTokenResponse,
    ValidateTokenRequest, ValidateTokenResponse,
    RefreshTokenRequest, RefreshTokenResponse,
    InvalidateTokenRequest, InvalidateTokenResponse,
    UserClaims,
};
use redis::aio::MultiplexedConnection;
use tonic::{Request, Response, Status};
use tracing::{info, error, debug};
use uuid::Uuid;
use crate::repository::token_repository::TokenRepository;

/// 认证服务实现
pub struct AuthServiceImpl {
    config: AppConfig,
    token_repository: TokenRepository,
}

impl AuthServiceImpl {
    pub fn new(config: AppConfig, redis_conn: MultiplexedConnection) -> Self {
        Self {
            config,
            token_repository: TokenRepository::new(redis_conn),
        }
    }
    
    /// 生成令牌对
    async fn generate_token_pair(&self, user_id: &str, username: &str) -> Result<(String, String, i64)> {
        // 生成访问令牌
        let access_token = utils::generate_jwt(&Uuid::parse_str(user_id)?, username)?;
        
        // 生成刷新令牌
        let refresh_token = Uuid::new_v4().to_string();
        
        // 访问令牌有效期
        let expires_in = self.config.jwt.expiration as i64;
        
        // 存储访问令牌
        self.token_repository
            .store_access_token(user_id, &access_token, expires_in)
            .await?;
        
        // 存储刷新令牌，有效期比访问令牌长
        let refresh_expires_in = expires_in * 2;
        self.token_repository
            .store_refresh_token(user_id, &refresh_token, refresh_expires_in)
            .await?;
        
        Ok((access_token, refresh_token, expires_in))
    }
}

#[tonic::async_trait]
impl AuthService for AuthServiceImpl {
    async fn validate_token(
        &self,
        request: Request<ValidateTokenRequest>,
    ) -> std::result::Result<Response<ValidateTokenResponse>, Status> {
        let req = request.into_inner();
        debug!("验证令牌请求");

        // 首先从Redis中验证令牌是否有效
        let user_id = match self.token_repository.validate_access_token(&req.token).await {
            Ok(Some(user_id)) => user_id,
            Ok(None) => {
                debug!("令牌无效或已过期");
                return Ok(Response::new(ValidateTokenResponse {
                    valid: false,
                    user_claims: None,
                }));
            }
            Err(err) => {
                error!("验证令牌时发生错误: {}", err);
                return Err(err.into());
            }
        };

        // 然后验证JWT的有效性
        let claims = match utils::validate_jwt(&req.token) {
            Ok(claims) => claims,
            Err(err) => {
                error!("JWT验证失败: {}", err);
                return Ok(Response::new(ValidateTokenResponse {
                    valid: false,
                    user_claims: None,
                }));
            }
        };

        debug!("令牌有效，用户ID: {}", user_id);

        // 返回响应
        Ok(Response::new(ValidateTokenResponse {
            valid: true,
            user_claims: Some(UserClaims {
                user_id: claims.sub,
                username: claims.username,
            }),
        }))
    }

    async fn create_token(
        &self,
        request: Request<CreateTokenRequest>,
    ) -> std::result::Result<Response<CreateTokenResponse>, Status> {
        let req = request.into_inner();
        debug!("创建令牌请求，用户ID: {}", req.user_id);

        // 生成令牌对
        let (access_token, refresh_token, expires_in) = match self
            .generate_token_pair(&req.user_id, &req.username)
            .await
        {
            Ok(tokens) => tokens,
            Err(err) => {
                error!("生成令牌对失败: {}", err);
                return Err(err.into());
            }
        };

        info!("成功为用户 {} 创建令牌", req.user_id);

        // 返回响应
        Ok(Response::new(CreateTokenResponse {
            access_token,
            refresh_token,
            expires_in,
        }))
    }
    
    async fn refresh_token(
        &self,
        request: Request<RefreshTokenRequest>,
    ) -> std::result::Result<Response<RefreshTokenResponse>, Status> {
        let req = request.into_inner();
        debug!("刷新令牌请求");
        
        // 验证刷新令牌
        let user_id = match self.token_repository.validate_refresh_token(&req.refresh_token).await {
            Ok(Some(user_id)) => user_id,
            Ok(None) => {
                debug!("刷新令牌无效或已过期");
                return Err(common::Error::TonicStatus(Status::unauthenticated("刷新令牌无效或已过期")).into());
            }
            Err(err) => {
                error!("验证刷新令牌时发生错误: {}", err);
                return Err(err.into());
            }
        };
        
        // 从用户ID获取用户名（实际中应调用user-service）
        // 简化起见，这里假设从JWT提取的用户ID已经足够
        // 在实际实现中，应该调用user-service获取用户信息
        
        // 生成新的令牌对
        let (access_token, refresh_token, expires_in) = match utils::validate_jwt(&req.refresh_token) {
            Ok(claims) => {
                match self.generate_token_pair(&user_id, &claims.username).await {
                    Ok(tokens) => tokens,
                    Err(err) => {
                        error!("生成新令牌对失败: {}", err);
                        return Err(err.into());
                    }
                }
            },
            Err(_) => {
                // 如果无法从刷新令牌中提取用户名，则假设为空字符串
                // 实际应用中应从用户服务获取
                match self.generate_token_pair(&user_id, "").await {
                    Ok(tokens) => tokens,
                    Err(err) => {
                        error!("生成新令牌对失败: {}", err);
                        return Err(err.into());
                    }
                }
            }
        };
        
        info!("成功为用户 {} 刷新令牌", user_id);
        
        // 返回响应
        Ok(Response::new(RefreshTokenResponse {
            access_token,
            refresh_token,
            expires_in,
        }))
    }
    
    async fn invalidate_token(
        &self,
        request: Request<InvalidateTokenRequest>,
    ) -> std::result::Result<Response<InvalidateTokenResponse>, Status> {
        let req = request.into_inner();
        debug!("注销令牌请求");
        
        // 使令牌失效
        let success = match self.token_repository.invalidate_token(&req.token).await {
            Ok(success) => success,
            Err(err) => {
                error!("使令牌失效时发生错误: {}", err);
                return Err(err.into());
            }
        };
        
        debug!("令牌注销结果: {}", success);
        
        // 返回响应
        Ok(Response::new(InvalidateTokenResponse { success }))
    }
} 