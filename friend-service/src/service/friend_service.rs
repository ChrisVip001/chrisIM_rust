use common::proto::friend::{
    SendFriendRequestRequest, AcceptFriendRequestRequest, RejectFriendRequestRequest,
    GetFriendListRequest, GetFriendRequestsRequest, DeleteFriendRequest, DeleteFriendResponse,
    CheckFriendshipRequest, CheckFriendshipResponse, FriendshipResponse, GetFriendListResponse,
    GetFriendRequestsResponse,
};
use common::proto::friend::friend_service_server::FriendService;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use uuid::Uuid;
use tracing::{info, error};

use crate::repository::friendship_repository::FriendshipRepository;

pub struct FriendServiceImpl {
    repository: FriendshipRepository,
}

impl FriendServiceImpl {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: FriendshipRepository::new(pool),
        }
    }
}

#[tonic::async_trait]
impl FriendService for FriendServiceImpl {
    // 发送好友请求
    async fn send_friend_request(
        &self,
        request: Request<SendFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let friend_id = req.friend_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;
        
        // 检查是否已存在好友关系
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(Some(_)) => {
                return Err(Status::already_exists("已经存在好友关系或请求"));
            }
            Ok(None) => {}
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }
        
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
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let friend_id = req.friend_id.parse::<Uuid>()
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
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let friend_id = req.friend_id.parse::<Uuid>()
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
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        match self.repository.get_friend_list(user_id).await {
            Ok(friends) => {
                let proto_friends = friends.into_iter()
                    .map(|f| f.to_proto())
                    .collect();
                
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
    
    // 获取好友请求列表
    async fn get_friend_requests(
        &self,
        request: Request<GetFriendRequestsRequest>,
    ) -> Result<Response<GetFriendRequestsResponse>, Status> {
        let req = request.into_inner();
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        match self.repository.get_friend_requests(user_id).await {
            Ok(requests) => {
                let proto_requests = requests.into_iter()
                    .map(|r| r.to_proto())
                    .collect();
                
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
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let friend_id = req.friend_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;
        
        match self.repository.delete_friend(user_id, friend_id).await {
            Ok(success) => {
                Ok(Response::new(DeleteFriendResponse {
                    success,
                }))
            }
            Err(e) => {
                error!("删除好友失败: {}", e);
                Err(Status::internal("删除好友失败"))
            }
        }
    }
    
    // 检查好友关系
    async fn check_friendship(
        &self,
        request: Request<CheckFriendshipRequest>,
    ) -> Result<Response<CheckFriendshipResponse>, Status> {
        let req = request.into_inner();
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let friend_id = req.friend_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;
        
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(status) => {
                Ok(Response::new(CheckFriendshipResponse {
                    status: status.unwrap_or_default() as i32,
                }))
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                Err(Status::internal("检查好友关系失败"))
            }
        }
    }
}