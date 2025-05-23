use common::proto::friend::friend_service_server::FriendService;
use common::proto::friend::{
    AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse,
    DeleteFriendRequest, DeleteFriendResponse, FriendshipResponse, GetFriendListRequest,
    GetFriendListResponse, GetFriendRequestsRequest, GetFriendRequestsResponse,
    RejectFriendRequestRequest, SendFriendRequestRequest,FriendshipStatus,
    UnblockUserRequest,BlockUserRequest,UnblockUserResponse,BlockUserResponse,
    CreateOrUpdateFriendGroupRequest, FriendGroupResponse, DeleteFriendGroupRequest,
    DeleteFriendGroupResponse, GetFriendGroupsRequest, GetFriendGroupsResponse,
    GetGroupFriendsRequest, GetGroupFriendsResponse,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use uuid::Uuid;

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

    // 检查用户是否存在的辅助方法
    async fn check_user_exists(&self, user_id: Uuid) -> Result<(), Status> {
        match self.repository.check_user_exists(user_id).await {
            Ok(user_exists) => {
                if !user_exists {
                    return Err(Status::not_found("用户不存在"));
                }
                Ok(())
            }
            Err(e) => {
                error!("检查用户是否存在失败: {}", e);
                Err(Status::internal("内部服务错误"))
            }
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

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        let message = &req.message;
        let msg_length = message.chars().count();
        if msg_length > 255 {
            return Err(Status::invalid_argument(
                format!("消息长度不能超过255个字符，当前长度: {}", msg_length)
            ));
        }
        
        // 检查用户和好友是否存在
        self.check_user_exists(user_id).await?;
        self.check_user_exists(friend_id).await?;

        // 检查是否已存在好友关系
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(Some(status)) => {
                // 如果状态是Pending或Accepted，则不允许重复发送请求
                // 如果是Rejected，则允许重新发送请求
                match status {
                    FriendshipStatus::Pending | FriendshipStatus::Accepted => {
                        return Err(Status::already_exists("已经存在好友关系或请求"));
                    }
                    FriendshipStatus::Rejected | FriendshipStatus::Expired => {
                        match self.repository.delete_friend(user_id, friend_id).await{
                            Ok(_) => {}
                            Err(e) => {
                                error!("删除好友关系失败: {}", e);
                                return Err(Status::internal("内部服务错误"));
                            }
                        }
                    }
                    FriendshipStatus::Blocked => {
                        return Err(Status::already_exists("好友关系已被锁定"));
                    }
                }
                // 对于Rejected状态，允许重新发送请求
            },
            Ok(None) => {},
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }

        // 创建好友请求
        match self
            .repository
            .create_friend_request(user_id, friend_id, message.to_string())
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

    // 接受好友请求
    async fn accept_friend_request(
        &self,
        request: Request<AcceptFriendRequestRequest>,
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

        // 检查好友请求是否存在
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(Some(status)) => {
                if status != FriendshipStatus::Pending {
                    return Err(Status::failed_precondition(
                        "无法接受的好友请求：不是处于等待状态",
                    ));
                }
            }
            Ok(None) => {
                return Err(Status::not_found("好友请求不存在"));
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }

        match self
            .repository
            .accept_friend_request(user_id, friend_id)
            .await
        {
            Ok(friendship) => {
                info!("接受好友请求成功，已建立双向好友关系: {:?}", friendship);
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

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        // 获取拒绝理由（如果有）
        let reason = if !req.reason.is_empty() {
            Some(req.reason)
        } else {
            None
        };

        // 检查好友请求是否存在且为待处理状态
        match self.repository.check_friendship(friend_id, user_id).await {
            Ok(Some(status)) => {
                if status != FriendshipStatus::Pending {
                    return Err(Status::failed_precondition(
                        "无法拒绝的好友请求：不是处于等待状态",
                    ));
                }
            }
            Ok(None) => {
                return Err(Status::not_found("好友请求不存在"));
            }
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }

        match self
            .repository
            .reject_friend_request(user_id, friend_id, reason)
            .await
        {
            Ok(friendship) => {
                info!("拒绝好友请求成功: {:?}", friendship);
                Ok(Response::new(FriendshipResponse {
                    friendship: Some(friendship.to_proto()),
                }))
            }
            Err(e) => {
                error!("拒绝好友请求失败: {}", e);
                Err(Status::internal(format!("拒绝好友请求失败: {}", e)))
            }
        }
    }

    // 获取好友列表
    async fn get_friend_list(
        &self,
        request: Request<GetFriendListRequest>,
    ) -> Result<Response<GetFriendListResponse>, Status> {
        let req = request.into_inner();

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        // 解析可选参数
        let page = if req.page > 0 { Some(req.page) } else { None };
        let page_size = if req.page_size > 0 { Some(req.page_size) } else { None };
        let sort_by = if req.sort_by.is_empty() { None } else { Some(req.sort_by) };

        let total = self.repository.count_friends(user_id).await.map_err(|e| {
            error!("获取好友总数失败: {}", e);
            Status::internal("获取好友总数失败")
        })?;
        let friends = self.repository.get_friend_list(user_id, page, page_size, sort_by).await.map_err(|e| {
            error!("获取好友列表失败: {}", e);
            Status::internal("获取好友列表失败")
        })?;
        let proto_friends = friends.into_iter().map(|f| f.to_proto()).collect();

        Ok(Response::new(GetFriendListResponse {
            friends: proto_friends,
            total,
        }))
    }

    // 获取好友请求列表
    async fn get_friend_requests(
        &self,
        request: Request<GetFriendRequestsRequest>,
    ) -> Result<Response<GetFriendRequestsResponse>, Status> {
        let req = request.into_inner();

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let page = if req.page > 0 { Some(req.page) } else { None };
        let page_size = if req.page_size > 0 { Some(req.page_size) } else { None };

        let total = self.repository.count_friend_requests(user_id).await.map_err(|e| {
            error!("获取好友请求总数失败: {}", e);
            Status::internal("获取好友请求总数失败")
        })?;
        let requests = self.repository.get_friend_requests(user_id, page, page_size).await.map_err(|e| {
            error!("获取好友请求列表失败: {}", e);
            Status::internal("获取好友请求列表失败")
        })?;
        let proto_requests = requests.into_iter().map(|r| r.to_proto()).collect();

        Ok(Response::new(GetFriendRequestsResponse {
            requests: proto_requests,
            total,
        }))
    }

    // 删除好友
    async fn delete_friend(
        &self,
        request: Request<DeleteFriendRequest>,
    ) -> Result<Response<DeleteFriendResponse>, Status> {
        let req = request.into_inner();

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        match self.repository.delete_friend(user_id, friend_id).await {
            Ok(success) => Ok(Response::new(DeleteFriendResponse { success })),
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

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let friend_id = req
            .friend_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(status) => Ok(Response::new(CheckFriendshipResponse {
                status: status.unwrap_or_default() as i32,
            })),
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                Err(Status::internal("检查好友关系失败"))
            }
        }
    }

    // 拉黑用户
    async fn block_user(
        &self,
        request: Request<BlockUserRequest>,
    ) -> Result<Response<BlockUserResponse>, Status> {
        let req = request.into_inner();

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let blocked_user_id = req
            .blocked_user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的被拉黑用户ID: {}", e)))?;

        // 检查用户是否存在
        self.check_user_exists(user_id).await?;
        self.check_user_exists(blocked_user_id).await?;

        // 检查是否已经拉黑
        if self.repository.is_user_blocked(user_id, blocked_user_id).await.map_err(|e| {
            error!("检查用户是否被拉黑失败: {}", e);
            Status::internal("检查用户是否被拉黑失败")
        })? {
            return Err(Status::already_exists("该用户已被拉黑"));
        }

        // 执行拉黑操作
        match self.repository.block_user(user_id, blocked_user_id).await {
            Ok(success) => {
                info!("用户 {} 成功拉黑用户 {}", user_id, blocked_user_id);
                Ok(Response::new(BlockUserResponse { success }))
            }
            Err(e) => {
                error!("拉黑用户失败: {}", e);
                Err(Status::internal("拉黑用户失败"))
            }
        }
    }

    // 解除拉黑
    async fn unblock_user(
        &self,
        request: Request<UnblockUserRequest>,
    ) -> Result<Response<UnblockUserResponse>, Status> {
        let req = request.into_inner();

        let user_id = req
            .user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        let blocked_user_id = req
            .blocked_user_id
            .parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的被解除拉黑用户ID: {}", e)))?;

        // 检查用户是否存在
        self.check_user_exists(user_id).await?;
        self.check_user_exists(blocked_user_id).await?;

        // 检查是否已经拉黑
        if !self.repository.is_user_blocked(user_id, blocked_user_id).await.map_err(|e| {
            error!("检查用户是否被拉黑失败: {}", e);
            Status::internal("检查用户是否被拉黑失败")
        })? {
            return Err(Status::not_found("该用户未被拉黑"));
        }

        // 执行解除拉黑操作
        match self.repository.unblock_user(user_id, blocked_user_id).await {
            Ok(success) => {
                info!("用户 {} 成功解除拉黑用户 {}", user_id, blocked_user_id);
                Ok(Response::new(UnblockUserResponse { success }))
            }
            Err(e) => {
                error!("解除拉黑用户失败: {}", e);
                Err(Status::internal("解除拉黑用户失败"))
            }
        }
    }

    // 创建或更新好友分组
    async fn create_or_update_friend_group(
        &self,
        request: Request<CreateOrUpdateFriendGroupRequest>,
    ) -> Result<Response<FriendGroupResponse>, Status> {
        let req = request.into_inner();
        let user_id = Uuid::parse_str(&req.user_id)
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        // 检查用户是否存在
        self.check_user_exists(user_id).await?;
        
        // 检查分组名称是否重复
        let exclude_group_id = req.id.as_ref().map(|id| Uuid::parse_str(id).unwrap());
        if self.repository.check_group_name_exists(user_id, &req.group_name, exclude_group_id).await.map_err(|e| {
            error!("检查分组名称是否重复失败: {}", e);
            Status::internal("检查分组名称是否重复失败")
        })? {
            return Err(Status::already_exists("分组名称已存在"));
        }
        // 解析并验证好友ID列表
        let friend_ids: Vec<Uuid> = req.friend_ids
            .into_iter()
            .map(|id| Uuid::parse_str(&id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Status::invalid_argument(format!("无效的好友ID: {}", e)))?;

        // 检查所有好友是否存在
        for friend_id in &friend_ids {
            self.check_user_exists(*friend_id).await?;
        }

        // 创建或更新分组
        let group = match req.id {
            Some(id) => {
                let group_id = Uuid::parse_str(&id)
                    .map_err(|e| Status::invalid_argument(format!("无效的分组ID: {}", e)))?;
                self.repository
                    .update_friend_group(group_id, user_id, req.group_name, req.sort_order)
                    .await
                    .map_err(|e| {
                        error!("更新好友分组失败: {}", e);
                        Status::internal("更新好友分组失败")
                    })?
            }
            None => {
                self.repository
                    .create_friend_group(user_id, req.group_name, req.sort_order)
                    .await
                    .map_err(|e| {
                        error!("创建好友分组失败: {}", e);
                        Status::internal("创建好友分组失败")
                    })?
            }
        };

        // 更新分组好友关系
        let updated_friend_ids = self.repository
            .update_group_friends(group.id, user_id, friend_ids)
            .await
            .map_err(|e| {
                error!("更新分组好友关系失败: {}", e);
                Status::internal("更新分组好友关系失败")
            })?;
        let mut friend_group = group.to_proto();
        friend_group.friend_count = updated_friend_ids.len() as i32;
        Ok(Response::new(FriendGroupResponse {
            group: Some(friend_group),
            friend_ids: updated_friend_ids.into_iter().map(|id| id.to_string()).collect(),
        }))
    }

    // 删除好友分组
    async fn delete_friend_group(
        &self,
        request: Request<DeleteFriendGroupRequest>,
    ) -> Result<Response<DeleteFriendGroupResponse>, Status> {
        let req = request.into_inner();
        let id = Uuid::parse_str(&req.id)
            .map_err(|e| Status::invalid_argument(format!("无效的分组ID: {}", e)))?;
        let user_id = Uuid::parse_str(&req.user_id)
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        self.check_user_exists(user_id).await?;

        let success = self.repository
            .delete_friend_group(id, user_id)
            .await
            .map_err(|e| {
                error!("删除好友分组失败: {}", e);
                Status::internal("删除好友分组失败")
            })?;

        Ok(Response::new(DeleteFriendGroupResponse { success }))
    }

    // 获取好友分组列表
    async fn get_friend_groups(
        &self,
        request: Request<GetFriendGroupsRequest>,
    ) -> Result<Response<GetFriendGroupsResponse>, Status> {
        let user_id = Uuid::parse_str(&request.into_inner().user_id)
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        self.check_user_exists(user_id).await?;

        let groups = self.repository
            .get_friend_groups(user_id)
            .await
            .map_err(|e| {
                error!("获取好友分组列表失败: {}", e);
                Status::internal("获取好友分组列表失败")
            })?;

        Ok(Response::new(GetFriendGroupsResponse {
            groups: groups.into_iter().map(|g| g.to_proto()).collect(),
        }))
    }

    // 获取分组好友列表
    async fn get_group_friends(
        &self,
        request: Request<GetGroupFriendsRequest>,
    ) -> Result<Response<GetGroupFriendsResponse>, Status> {
        let req = request.into_inner();
        
        let group_id = Uuid::parse_str(&req.group_id)
            .map_err(|e| Status::invalid_argument(format!("无效的分组ID: {}", e)))?;
            
        let user_id = Uuid::parse_str(&req.user_id)
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;

        // 检查用户是否存在
        self.check_user_exists(user_id).await?;

        // 获取分组好友列表
        let friends = self.repository
            .get_group_friends(group_id, user_id)
            .await
            .map_err(|e| {
                error!("获取分组好友列表失败: {}", e);
                Status::internal("获取分组好友列表失败")
            })?;

        let total = friends.len() as i32;
        Ok(Response::new(GetGroupFriendsResponse {
            friends: friends.into_iter().map(|f| f.to_proto()).collect(),
            total,
        }))
    }
}
