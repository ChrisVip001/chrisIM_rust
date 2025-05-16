use anyhow::Result;
use common::proto::friend::friend_service_server::FriendService;
use common::proto::friend::{
    AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse,
    DeleteFriendRequest, DeleteFriendResponse, FriendshipResponse, GetFriendListRequest,
    GetFriendListResponse, GetFriendRequestsRequest, GetFriendRequestsResponse,
    RejectFriendRequestRequest, SendFriendRequestRequest,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use uuid::Uuid;

use crate::repository::friendship_repository::FriendshipRepository;

// 导入宏
use common::generate_grpc_client;

// 使用宏生成user-service客户端
generate_grpc_client!(
    name: UserServiceGrpcClient, 
    service: "user-service",
    proto_path: common::proto::user,
    client_type: user_service_client::UserServiceClient,
    methods: [
        get_user_by_id(GetUserByIdRequest) -> UserResponse,
        get_user_by_username(GetUserByUsernameRequest) -> UserResponse,
        check_user_status(CheckUserStatusRequest) -> CheckUserStatusResponse
    ]
);

pub struct FriendServiceImplWithMacro {
    repository: FriendshipRepository,
    user_client: UserServiceGrpcClient,
}

impl FriendServiceImplWithMacro {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: FriendshipRepository::new(pool),
            user_client: UserServiceGrpcClient::from_env(),
        }
    }
    
    // 添加辅助方法：验证用户是否存在且状态正常
    async fn validate_user(&self, user_id: &str) -> Result<(), Status> {
        match self.user_client.check_user_status(CheckUserStatusRequest {
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                if !response.exists {
                    return Err(Status::not_found(format!("用户 {} 不存在", user_id)));
                }
                
                if !response.is_active {
                    return Err(Status::permission_denied(format!(
                        "用户 {} 状态异常: {:?}", 
                        user_id, 
                        response.status
                    )));
                }
                
                // 检查具体状态
                use common::proto::user::UserStatus;
                match response.status {
                    UserStatus::Banned => {
                        return Err(Status::permission_denied(format!("用户 {} 已被禁用", user_id)));
                    }
                    UserStatus::Deleted => {
                        return Err(Status::not_found(format!("用户 {} 已被删除", user_id)));
                    }
                    UserStatus::Inactive => {
                        return Err(Status::permission_denied(format!("用户 {} 未激活", user_id)));
                    }
                    UserStatus::Active => {
                        // 正常状态，继续处理
                    }
                }
                
                Ok(())
            }
            Err(e) => {
                error!("验证用户状态失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
        }
    }
}

#[tonic::async_trait]
impl FriendService for FriendServiceImplWithMacro {
    // 发送好友请求，并使用user-service验证用户存在
    async fn send_friend_request(
        &self,
        request: Request<SendFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        // 使用辅助方法验证请求用户
        self.validate_user(&req.user_id).await?;
        
        // 验证好友用户
        self.validate_user(&req.friend_id).await?;

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
        match self
            .repository
            .create_friend_request(user_id, friend_id)
            .await
        {
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

    // 接受好友请求实现
    async fn accept_friend_request(
        &self,
        request: Request<AcceptFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();

        // 验证双方用户状态
        self.validate_user(&req.user_id).await?;
        self.validate_user(&req.friend_id).await?;

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        match self
            .repository
            .accept_friend_request(user_id, friend_id)
            .await
        {
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

    // 其他方法实现...
    // 以下是方法存根
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