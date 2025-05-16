use common::proto::friend::friend_service_server::FriendService;
use common::proto::friend::{
    AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse,
    DeleteFriendRequest, DeleteFriendResponse, FriendshipResponse, GetFriendListRequest,
    GetFriendListResponse, GetFriendRequestsRequest, GetFriendRequestsResponse,
    RejectFriendRequestRequest, SendFriendRequestRequest,
};
use common::validation::{FriendValidator, UserValidator, CompositeValidator, Validator};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use uuid::Uuid;

use crate::repository::friendship_repository::FriendshipRepository;

/// 使用通用验证框架的好友服务实现
pub struct FriendServiceWithValidation {
    repository: FriendshipRepository,
    user_validator: UserValidator,
    friend_validator: FriendValidator,
}

impl FriendServiceWithValidation {
    /// 创建新的服务实例
    pub fn new(pool: PgPool) -> Self {
        let user_validator = UserValidator::new();
        let friend_validator = FriendValidator::new()
            .with_user_validator(user_validator.clone());
            
        Self {
            repository: FriendshipRepository::new(pool),
            user_validator,
            friend_validator,
        }
    }
    
    /// 创建组合验证器
    fn create_validator(&self) -> CompositeValidator<Box<dyn Validator + Send + Sync>> {
        let mut validator = CompositeValidator::new();
        
        // 添加用户验证器
        validator.add_validator(Box::new(self.user_validator.clone()));
        
        // 添加好友验证器  
        validator.add_validator(Box::new(self.friend_validator.clone()));
        
        validator
    }
}

#[tonic::async_trait]
impl FriendService for FriendServiceWithValidation {
    // 发送好友请求
    async fn send_friend_request(
        &self,
        request: Request<SendFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();
        
        // 使用独立验证
        self.friend_validator.validate_can_be_friends(&req.user_id, &req.friend_id).await?;
        
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
        
        // 使用组合验证器 - 使用can_perform方法
        self.create_validator()
            .validate_all("accept_friend", &req.user_id, Some(&req.friend_id))
            .await?;
            
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

    // 拒绝好友请求
    async fn reject_friend_request(
        &self,
        request: Request<RejectFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();
        
        // 使用显式验证
        self.user_validator.validate_user_status(&req.user_id).await?;
        self.user_validator.validate_user_status(&req.friend_id).await?;
        
        self.friend_validator.validate_has_pending_request(&req.friend_id, &req.user_id).await?;
        
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        match self.repository.reject_friend_request(user_id, friend_id).await {
            Ok(friendship) => {
                info!("拒绝好友请求成功: {:?}", friendship);
                Ok(Response::new(FriendshipResponse {
                    friendship: Some(friendship.to_proto()),
                }))
            }
            Err(e) => {
                error!("拒绝好友请求失败: {}", e);
                Err(Status::internal("拒绝好友请求失败"))
            }
        }
    }

    // 获取好友列表
    async fn get_friend_list(
        &self,
        request: Request<GetFriendListRequest>,
    ) -> Result<Response<GetFriendListResponse>, Status> {
        let req = request.into_inner();
        
        // 仅验证用户状态
        self.user_validator.validate_user_status(&req.user_id).await?;
        
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
            
        match self.repository.get_friend_list(user_id).await {
            Ok(friends) => {
                // 将数据库实体转换为proto格式
                let proto_friends = friends.into_iter().map(|f| f.to_proto()).collect();
                
                Ok(Response::new(GetFriendListResponse {
                    friends: proto_friends,
                }))
            }
            Err(e) => {
                error!("获取好友列表失败: {}", e);
                Err(Status::internal("获取好友列表失败"))
            }
        }
    }

    // 其他方法实现...
    // 获取好友请求列表
    async fn get_friend_requests(
        &self,
        request: Request<GetFriendRequestsRequest>,
    ) -> Result<Response<GetFriendRequestsResponse>, Status> {
        let req = request.into_inner();
        
        // 验证用户
        self.user_validator.validate_user_status(&req.user_id).await?;
        
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
            
        match self.repository.get_friend_requests(user_id).await {
            Ok(requests) => {
                let proto_requests = requests.into_iter().map(|r| r.to_proto()).collect();
                
                Ok(Response::new(GetFriendRequestsResponse {
                    requests: proto_requests,
                }))
            }
            Err(e) => {
                error!("获取好友请求列表失败: {}", e);
                Err(Status::internal("获取好友请求列表失败"))
            }
        }
    }

    // 删除好友
    async fn delete_friend(
        &self,
        request: Request<DeleteFriendRequest>,
    ) -> Result<Response<DeleteFriendResponse>, Status> {
        let req = request.into_inner();
        
        // 使用组合验证器
        self.create_validator()
            .validate_all("remove_friend", &req.user_id, Some(&req.friend_id))
            .await?;
            
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;
            
        match self.repository.delete_friendship(user_id, friend_id).await {
            Ok(_) => {
                info!("删除好友关系成功: {} 和 {}", user_id, friend_id);
                Ok(Response::new(DeleteFriendResponse { success: true }))
            }
            Err(e) => {
                error!("删除好友关系失败: {}", e);
                Err(Status::internal("删除好友关系失败"))
            }
        }
    }

    // 检查好友关系
    async fn check_friendship(
        &self,
        request: Request<CheckFriendshipRequest>,
    ) -> Result<Response<CheckFriendshipResponse>, Status> {
        let req = request.into_inner();
        
        // 这个操作只需要验证用户存在
        self.user_validator.validate_user_status(&req.user_id).await?;
        self.user_validator.validate_user_status(&req.friend_id).await?;
        
        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;
            
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(Some(friendship)) => {
                Ok(Response::new(CheckFriendshipResponse {
                    status: friendship.status,
                }))
            }
            Ok(None) => {
                // 不存在关系，返回默认状态
                Ok(Response::new(CheckFriendshipResponse {
                    status: 0, // 没有关系
                }))
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                Err(Status::internal("检查好友关系失败"))
            }
        }
    }
} 