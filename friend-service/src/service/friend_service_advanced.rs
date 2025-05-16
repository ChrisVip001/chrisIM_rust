use std::time::Duration;
use std::sync::Arc;

use common::proto::friend::friend_service_server::FriendService;
use common::proto::friend::{
    AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse,
    DeleteFriendRequest, DeleteFriendResponse, FriendshipResponse, GetFriendListRequest,
    GetFriendListResponse, GetFriendRequestsRequest, GetFriendRequestsResponse,
    RejectFriendRequestRequest, SendFriendRequestRequest,
};
use common::validation::{FriendValidator, UserValidator, ValidationMiddleware};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use uuid::Uuid;

use crate::repository::friendship_repository::FriendshipRepository;

/// 使用高级验证中间件的好友服务
pub struct FriendServiceAdvanced {
    repository: FriendshipRepository,
    user_validator: UserValidator,
    friend_validator: FriendValidator,
    validation_middleware: Arc<ValidationMiddleware>,
}

impl FriendServiceAdvanced {
    /// 创建新的服务实例
    pub fn new(pool: PgPool) -> Self {
        let user_validator = UserValidator::new();
        let friend_validator = FriendValidator::new()
            .with_user_validator(user_validator);
        
        // 配置验证中间件
        let validation_middleware = Arc::new(
            ValidationMiddleware::new()
                .with_cache_ttl(Duration::from_secs(30)) // 缓存30秒
                .with_rate_limit(Duration::from_secs(60), 50) // 每分钟最多50次调用
        );
            
        Self {
            repository: FriendshipRepository::new(pool),
            user_validator,
            friend_validator,
            validation_middleware,
        }
    }
}

#[tonic::async_trait]
impl FriendService for FriendServiceAdvanced {
    // 发送好友请求
    async fn send_friend_request(
        &self,
        request: Request<SendFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();
        
        // 使用带缓存的验证中间件
        self.validation_middleware.validate_and_log(
            "add_friend", 
            &req.user_id, 
            Some(&req.friend_id),
            || {
                // 闭包内执行实际验证逻辑
                // 中间件会自动处理缓存和限流
                async move {
                    self.friend_validator.validate_can_be_friends(&req.user_id, &req.friend_id).await
                }.into()
            }
        ).await?;
        
        // 解析ID
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;
            
        // 创建好友请求
        match self.repository.create_friend_request(user_id, friend_id).await {
            Ok(friendship) => {
                // 操作成功后，清除对应的验证缓存，确保下次查询获取最新状态
                let cache_key = format!("add_friend:{}:{}", req.user_id, req.friend_id);
                self.validation_middleware.invalidate_cache(&cache_key).await;
                
                info!("创建好友请求成功: {:?}", friendship);
                Ok(Response::new(FriendshipResponse {
                    friendship: Some(friendship.to_proto()),
                }))
            }
            Err(e) => {
                error!("创建好友请求失败: {}", e);
                Err(Status::internal("创建好友请求失败"))
            }
        }
    }

    // 接受好友请求
    async fn accept_friend_request(
        &self,
        request: Request<AcceptFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();
        
        // 验证请求
        self.validation_middleware.validate_and_log(
            "accept_friend",
            &req.user_id,
            Some(&req.friend_id),
            || {
                async move {
                    // 验证用户
                    self.user_validator.validate_user_status(&req.user_id).await?;
                    self.user_validator.validate_user_status(&req.friend_id).await?;
                    
                    // 验证是否有待处理请求
                    self.friend_validator.validate_has_pending_request(&req.friend_id, &req.user_id).await
                }.into()
            }
        ).await?;
        
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        match self.repository.accept_friend_request(user_id, friend_id).await {
            Ok(friendship) => {
                // 清除相关缓存
                let cache_key1 = format!("accept_friend:{}:{}", req.user_id, req.friend_id);
                let cache_key2 = format!("add_friend:{}:{}", req.user_id, req.friend_id);
                self.validation_middleware.invalidate_cache(&cache_key1).await;
                self.validation_middleware.invalidate_cache(&cache_key2).await;
                
                info!("接受好友请求成功: {:?}", friendship);
                Ok(Response::new(FriendshipResponse {
                    friendship: Some(friendship.to_proto()),
                }))
            }
            Err(e) => {
                error!("接受好友请求失败: {}", e);
                Err(Status::internal("接受好友请求失败"))
            }
        }
    }

    // 其他方法实现...（为了简洁，省略了其他方法）
    async fn reject_friend_request(
        &self,
        _request: Request<RejectFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        Err(Status::unimplemented("方法未实现"))
    }

    async fn get_friend_list(
        &self,
        _request: Request<GetFriendListRequest>,
    ) -> Result<Response<GetFriendListResponse>, Status> {
        Err(Status::unimplemented("方法未实现"))
    }

    async fn get_friend_requests(
        &self,
        _request: Request<GetFriendRequestsRequest>,
    ) -> Result<Response<GetFriendRequestsResponse>, Status> {
        Err(Status::unimplemented("方法未实现"))
    }

    async fn delete_friend(
        &self,
        _request: Request<DeleteFriendRequest>,
    ) -> Result<Response<DeleteFriendResponse>, Status> {
        Err(Status::unimplemented("方法未实现"))
    }

    async fn check_friendship(
        &self,
        _request: Request<CheckFriendshipRequest>,
    ) -> Result<Response<CheckFriendshipResponse>, Status> {
        Err(Status::unimplemented("方法未实现"))
    }
} 