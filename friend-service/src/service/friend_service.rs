use std::future::Future;
use common::proto::friend::friend_service_server::FriendService;
use common::proto::friend::{AcceptFriendRequestRequest, CheckFriendshipRequest, CheckFriendshipResponse, DeleteFriendRequest, DeleteFriendResponse, FriendshipResponse, GetFriendListRequest, GetFriendListResponse, GetFriendRequestsRequest, GetFriendRequestsResponse, RejectFriendRequestRequest, SendFriendRequestRequest, FriendshipStatus, UnblockUserRequest, BlockUserRequest, UnblockUserResponse, BlockUserResponse, CreateOrUpdateFriendGroupRequest, FriendGroupResponse, DeleteFriendGroupRequest, DeleteFriendGroupResponse, GetFriendGroupsRequest, GetFriendGroupsResponse, GetGroupFriendsRequest, GetGroupFriendsResponse, SearchPotentialFriendsRequest, SearchPotentialFriendsResponse, GetAllFriendDetailListRequest, GetAllFriendDetailListResponse, ToggleFriendStarRequest, ToggleFriendStarResponse, ToggleFriendTopRequest, ToggleFriendTopResponse, UpdateFriendRemarkRequest, UpdateFriendRemarkResponse, GetUserBlacklistRequest, GetUserBlacklistResponse, UserBlacklistWithInfo, IsBlockedRequest, IsBlockedResponse};
use anyhow;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use common::config::ConfigLoader;
use common::grpc_client::base::get_rpc_client;
use common::grpc_client::UserServiceGrpcClient;
use common::proto::user::user_service_client::UserServiceClient;
use common::service_discovery::LbWithServiceDiscovery;
use common::service_register_center::service_register_center;
use crate::repository::friendship_repository::FriendshipRepository;
use crate::model::friendship::{PotentialFriend, DetailedFriend};
use crate::model::user_blacklist::UserBlacklist;

pub struct FriendServiceImpl {
    repository: FriendshipRepository,
    service_client: UserServiceClient<LbWithServiceDiscovery>,
}

impl FriendServiceImpl {
    pub async fn new(pool: PgPool) -> anyhow::Result<Self> {
        let config = ConfigLoader::get_global().expect("Failed to get global config");

        let service_client = get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(&*config, "user".to_string()).await?;

        Ok(Self {
            repository: FriendshipRepository::new(pool),
            service_client,
        })
    }

    // 检查用户是否存在的辅助方法
    async fn check_user_exists(&self, user_id: &str) -> Result<(), Status> {
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

    // 添加好友星标状态辅助方法
    async fn toggle_friend_star_helper(&self, user_id: &str, friend_id: &str, is_starred: bool) -> Result<bool, Status> {
        // 检查用户是否存在
        match self.check_user_exists(user_id).await {
            Ok(_) => {},
            Err(e) => return Err(e),
        }
        
        // 检查好友关系
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(status) => {
                if status != Some(FriendshipStatus::Accepted) {
                    return Err(Status::failed_precondition("不是好友关系，无法设置星标状态"));
                }
            },
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }
        
        // 更新星标状态
        match self.repository.update_friend_star(user_id, friend_id, is_starred).await {
            Ok(success) => {
                if success {
                    Ok(true)
                } else {
                    Err(Status::internal("设置星标状态失败"))
                }
            },
            Err(e) => {
                error!("设置好友星标状态失败: {}", e);
                Err(Status::internal("设置星标状态失败"))
            }
        }
    }

    // 添加好友置顶状态辅助方法
    async fn toggle_friend_top_helper(&self, user_id: &str, friend_id: &str, is_top: bool) -> Result<bool, Status> {
        // 检查用户是否存在
        match self.check_user_exists(user_id).await {
            Ok(_) => {},
            Err(e) => return Err(e),
        }
        
        // 检查好友关系
        match self.repository.check_friendship(user_id, friend_id).await {
            Ok(status) => {
                if status != Some(FriendshipStatus::Accepted) {
                    return Err(Status::failed_precondition("不是好友关系，无法设置置顶状态"));
                }
            },
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }
        
        // 更新置顶状态
        match self.repository.update_friend_top(user_id, friend_id, is_top).await {
            Ok(success) => {
                if success {
                    Ok(true)
                } else {
                    Err(Status::internal("设置置顶状态失败"))
                }
            },
            Err(e) => {
                error!("设置好友置顶状态失败: {}", e);
                Err(Status::internal("设置置顶状态失败"))
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

        let user_id = req.user_id;
        let friend_id = req.friend_id;

        let message = &req.message;
        let msg_length = message.chars().count();
        if msg_length > 255 {
            return Err(Status::invalid_argument(
                format!("消息长度不能超过255个字符，当前长度: {}", msg_length)
            ));
        }
        
        // 检查用户和好友是否存在
        self.check_user_exists(&user_id).await?;
        self.check_user_exists(&friend_id).await?;

        // 检查是否已存在好友关系
        match self.repository.check_friendship(&user_id, &friend_id).await {
            Ok(Some(status)) => {
                // 如果状态是Pending或Accepted，则不允许重复发送请求
                // 如果是Rejected，则允许重新发送请求
                match status {
                    FriendshipStatus::Accepted => {
                        return Err(Status::already_exists("已经存在好友关系"));
                    }
                    FriendshipStatus::Pending | FriendshipStatus::Rejected | FriendshipStatus::Expired => {
                        match self.repository.delete_friend(&user_id, &friend_id).await{
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
            .create_friend_request(&user_id, &friend_id, message.to_string())
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

        let user_id = req.user_id;
        let request_id = req.request_id;

        // 检查请求ID是否为空
        if request_id.is_empty() {
            return Err(Status::invalid_argument("请求ID不能为空"));
        }

        match self
            .repository
            .accept_friend_request(&user_id, &request_id)
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
                Err(Status::internal(format!("接受好友请求失败: {}", e)))
            }
        }
    }

    // 拒绝好友请求
    async fn reject_friend_request(
        &self,
        request: Request<RejectFriendRequestRequest>,
    ) -> Result<Response<FriendshipResponse>, Status> {
        let req = request.into_inner();

        let user_id = req.user_id;
        let request_id = req.request_id;

        // 检查请求ID是否为空
        if request_id.is_empty() {
            return Err(Status::invalid_argument("请求ID不能为空"));
        }

        // 获取拒绝理由（如果有）
        let reason = if !req.reason.is_empty() {
            Some(req.reason)
        } else {
            None
        };

        match self
            .repository
            .reject_friend_request(&user_id, reason, &request_id)
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
        let user_id = req.user_id;

        // 解析可选参数
        let page = (req.page > 0).then_some(req.page);
        let page_size = (req.page_size > 0).then_some(req.page_size);
        
        // 解析排序方式和搜索关键词
        let sort_by = (!req.sort_by.is_empty()).then_some(req.sort_by.clone());
        let keyword = (!req.keyword.is_empty()).then_some(req.keyword.clone());

        // 获取总数
        let total = self.repository.count_friends(&user_id, keyword.clone()).await.map_err(|e| {
            error!("获取好友总数失败: {}", e);
            Status::internal("获取好友总数失败")
        })?;

        // 获取好友列表
        let friends = self.repository
            .get_friend_list(&user_id, page, page_size, sort_by, keyword.clone())
            .await
            .map_err(|e| {
                error!("获取好友列表失败: {}", e);
                Status::internal("获取好友列表失败")
            })?;

        // 转换为proto对象
        let friend_protos = friends.into_iter().map(|f| f.to_proto()).collect();

        Ok(Response::new(GetFriendListResponse {
            total,
            friends: friend_protos,
        }))
    }

    // 获取好友请求列表
    async fn get_friend_requests(
        &self,
        request: Request<GetFriendRequestsRequest>,
    ) -> Result<Response<GetFriendRequestsResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;

        // 解析可选参数
        let page = (req.page > 0).then_some(req.page);
        let page_size = (req.page_size > 0).then_some(req.page_size);

        // 获取请求总数
        let total = self.repository.count_friend_requests(&user_id).await.map_err(|e| {
            error!("获取好友请求总数失败: {}", e);
            Status::internal("获取好友请求总数失败")
        })?;

        // 获取请求列表
        let requests = self.repository
            .get_friend_requests(&user_id, page, page_size)
            .await
            .map_err(|e| {
                error!("获取好友请求列表失败: {}", e);
                Status::internal("获取好友请求列表失败")
            })?;

        // 转换为proto对象
        let request_protos = requests.into_iter().map(|r| r.to_proto()).collect();

        Ok(Response::new(GetFriendRequestsResponse {
            total,
            requests: request_protos,
        }))
    }

    // 删除好友
    async fn delete_friend(
        &self,
        request: Request<DeleteFriendRequest>,
    ) -> Result<Response<DeleteFriendResponse>, Status> {
        let req = request.into_inner();

        let user_id = req.user_id;
        let friend_id = req.friend_id;

        match self.repository.delete_friend(&user_id, &friend_id).await {
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

        let user_id = req.user_id;
        let friend_id = req.friend_id;

        match self.repository.check_friendship(&user_id, &friend_id).await {
            Ok(status) => {
                Ok(Response::new(CheckFriendshipResponse {
                    status: status.map(|s| s as i32).unwrap_or(0),
                }))
            }
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

        let user_id = req.user_id.clone();
        let blocked_user_id = req.blocked_user_id.clone();
        let reason = if req.reason.is_empty() { None } else { Some(req.reason) };

        // 检查用户是否存在
        self.check_user_exists(&user_id).await?;
        self.check_user_exists(&blocked_user_id).await?;

        // 检查是否已经拉黑
        if self.repository.is_user_blocked(&user_id, &blocked_user_id).await.map_err(|e| {
            error!("检查用户是否被拉黑失败: {}", e);
            Status::internal("检查用户是否被拉黑失败")
        })? {
            return Err(Status::already_exists("该用户已被拉黑"));
        }

        match self.repository.block_user(&user_id, &blocked_user_id, reason).await {
            Ok(blacklist) => {
                info!("用户 {} 成功拉黑用户 {}，好友关系状态已更新为拉黑（如果存在）", user_id, blocked_user_id);
                Ok(Response::new(BlockUserResponse {
                    blacklist: Some(blacklist.to_proto()),
                }))
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

        let user_id = req.user_id.clone();
        let blocked_user_id = req.blocked_user_id.clone();

        // 检查用户是否存在
        self.check_user_exists(&user_id).await?;
        self.check_user_exists(&blocked_user_id).await?;

        // 检查是否已经拉黑
        if !self.repository.is_user_blocked(&user_id, &blocked_user_id).await.map_err(|e| {
            error!("检查用户是否被拉黑失败: {}", e);
            Status::internal("检查用户是否被拉黑失败")
        })? {
            return Err(Status::failed_precondition("该用户未被拉黑"));
        }

        match self.repository.unblock_user(&user_id, &blocked_user_id).await {
            Ok(success) => {
                info!("用户 {} 成功解除拉黑用户 {}，好友关系已自动恢复（如果之前存在）", user_id, blocked_user_id);
                Ok(Response::new(UnblockUserResponse {
                    success,
                }))
            }
            Err(e) => {
                error!("解除拉黑失败: {}", e);
                Err(Status::internal("解除拉黑失败"))
            }
        }
    }

    // 创建或更新好友分组
    async fn create_or_update_friend_group(
        &self,
        request: Request<CreateOrUpdateFriendGroupRequest>,
    ) -> Result<Response<FriendGroupResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id.clone();

        // 检查用户是否存在
        self.check_user_exists(&user_id).await?;
        
        // 检查分组名称是否重复
        let exclude_group_id = req.id.as_ref().map(|id| id.clone());
        if self.repository.check_group_name_exists(&user_id, &req.group_name, exclude_group_id.clone()).await.map_err(|e| {
            error!("检查分组名称是否重复失败: {}", e);
            Status::internal("检查分组名称是否重复失败")
        })? {
            return Err(Status::already_exists("分组名称已存在"));
        }
        
        // 解析并验证好友ID列表
        let friend_ids: Vec<String> = req.friend_ids.clone();

        // 检查所有好友是否存在
        for friend_id in &friend_ids {
            self.check_user_exists(friend_id).await?;
        }

        // 创建或更新分组
        let group = match req.id {
            Some(id) => {
                self.repository
                    .update_friend_group(&id, &user_id, req.group_name, req.sort_order)
                    .await
                    .map_err(|e| {
                        error!("更新好友分组失败: {}", e);
                        Status::internal("更新好友分组失败")
                    })?
            }
            None => {
                self.repository
                    .create_friend_group(&user_id, req.group_name, req.sort_order)
                    .await
                    .map_err(|e| {
                        error!("创建好友分组失败: {}", e);
                        Status::internal("创建好友分组失败")
                    })?
            }
        };
        
        // 更新分组中的好友列表
        let updated_friend_ids = self.repository
            .update_group_friends(&group.id, &user_id, &friend_ids)
            .await
            .map_err(|e| {
                error!("更新好友分组中的好友失败: {}", e);
                Status::internal("更新好友分组中的好友失败")
            })?;
            
        // 转换为proto对象
        let mut friend_group = group.to_proto();
        friend_group.friend_count = updated_friend_ids.len() as i32;
        
        Ok(Response::new(FriendGroupResponse {
            group: Some(friend_group),
            friend_ids: updated_friend_ids,
        }))
    }

    // 删除好友分组
    async fn delete_friend_group(
        &self,
        request: Request<DeleteFriendGroupRequest>,
    ) -> Result<Response<DeleteFriendGroupResponse>, Status> {
        let req = request.into_inner();
        let id = req.id.clone();
        let user_id = req.user_id.clone();

        self.check_user_exists(&user_id).await?;

        match self.repository.delete_friend_group(&id, &user_id).await {
            Ok(success) => {
                Ok(Response::new(DeleteFriendGroupResponse { success }))
            }
            Err(e) => {
                error!("删除好友分组失败: {}", e);
                Err(Status::internal("删除好友分组失败"))
            }
        }
    }

    // 获取好友分组列表
    async fn get_friend_groups(
        &self,
        request: Request<GetFriendGroupsRequest>,
    ) -> Result<Response<GetFriendGroupsResponse>, Status> {
        let user_id = request.into_inner().user_id.clone();

        self.check_user_exists(&user_id).await?;

        let groups = self.repository.get_friend_groups(&user_id).await.map_err(|e| {
            error!("获取好友分组列表失败: {}", e);
            Status::internal("获取好友分组列表失败")
        })?;

        let group_protos = groups.into_iter().map(|g| g.to_proto()).collect();

        Ok(Response::new(GetFriendGroupsResponse {
            groups: group_protos,
        }))
    }

    // 获取分组下的好友列表
    async fn get_group_friends(
        &self,
        request: Request<GetGroupFriendsRequest>,
    ) -> Result<Response<GetGroupFriendsResponse>, Status> {
        let req = request.into_inner();
        
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();

        // 检查用户是否存在
        self.check_user_exists(&user_id).await?;

        let friends = self.repository.get_group_friends(&group_id, &user_id).await.map_err(|e| {
            error!("获取分组好友列表失败: {}", e);
            Status::internal("获取分组好友列表失败")
        })?;

        let friend_protos: Vec<_> = friends.into_iter().map(|f| f.to_proto()).collect();
        let total = friend_protos.len() as i32;

        Ok(Response::new(GetGroupFriendsResponse {
            friends: friend_protos,
            total,
        }))
    }

    // 搜索潜在好友
    async fn search_potential_friends(
        &self,
        request: Request<SearchPotentialFriendsRequest>,
    ) -> Result<Response<SearchPotentialFriendsResponse>, Status> {
        let req = request.into_inner();
        
        let user_id = req.user_id.clone();
        let search_term = req.search_term.clone();
        
        // 进行搜索
        let users = match self.repository.search_potential_friends(
            &user_id,
            &search_term
        ).await {
            Ok(result) => result,
            Err(e) => {
                error!("搜索潜在好友失败: {}", e);
                return Err(Status::internal("搜索潜在好友失败"));
            }
        };
        
        // 转换为PotentialFriend对象
        let mut potential_friends: Vec<_> = Vec::with_capacity(users.len());
        
        for (id, username, nickname, avatar_url, phone, friendship_status, sign) in users {
            // 使用默认隐私设置（这里固定为2表示不显示完整手机号）
            // 在实际生产环境中，可以从配置系统获取
            let show_phone = 2; // 2表示不显示手机号，1表示显示
            
            // 根据隐私配置处理手机号
            let new_phone = if let Some(phone_str) = phone {
                if show_phone == 2 { // 2表示不显示手机号
                    // 将手机号处理为脱敏状态
                    if !phone_str.is_empty() && phone_str.len() >= 7 {
                        let prefix = &phone_str[0..3];
                        let suffix = &phone_str[phone_str.len() - 4..];
                        let stars = "*".repeat(phone_str.len() - 7);
                        Some(format!("{}{}{}", prefix, stars, suffix))
                    } else {
                        Some(phone_str)
                    }
                } else {
                    Some(phone_str) // 保持原样显示
                }
            } else {
                None
            };
            
            let friend = PotentialFriend::from_tuple(
                id, username, nickname, avatar_url, new_phone, friendship_status, sign
            );
            potential_friends.push(friend.to_proto());
        }
        
        Ok(Response::new(SearchPotentialFriendsResponse {
            users: potential_friends,
        }))
    }

    /// 获取所有好友详细列表（无分页）
    async fn get_all_friend_detail_list(
        &self,
        request: Request<GetAllFriendDetailListRequest>,
    ) -> Result<Response<GetAllFriendDetailListResponse>, Status> {
        let user_id = request.into_inner().user_id;
        
        // 基本参数验证
        if user_id.is_empty() {
            return Err(Status::invalid_argument("用户ID不能为空"));
        }
        
        // 检查用户是否存在
        match self.check_user_exists(&user_id).await {
            Ok(_) => {},
            Err(e) => return Err(e),
        }
        
        // 获取所有好友详细列表
        match self.repository.get_all_friend_detail_list(&user_id).await {
            Ok(friends) => {
                // 转换为proto消息
                let proto_friends = friends.into_iter()
                    .map(|f| f.to_proto())
                    .collect();
                
                Ok(Response::new(GetAllFriendDetailListResponse {
                    friends: proto_friends,
                }))
            },
            Err(e) => {
                error!("获取好友详细列表失败: {}", e);
                Err(Status::internal("获取好友详细列表失败"))
            }
        }
    }

    // 添加好友星标状态
    async fn toggle_friend_star(
        &self,
        request: Request<ToggleFriendStarRequest>,
    ) -> Result<Response<ToggleFriendStarResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id.clone();
        let friend_id = req.friend_id.clone();
        let is_starred = req.is_starred;
        
        // 更新星标状态
        match self.toggle_friend_star_helper(&user_id, &friend_id, is_starred).await {
            Ok(success) => {
                Ok(Response::new(ToggleFriendStarResponse {
                    success: success,
                }))
            },
            Err(e) => {
                error!("设置好友星标状态失败: {:?}", e);
                Err(e)
            }
        }
    }

    // 添加好友置顶状态
    async fn toggle_friend_top(
        &self,
        request: Request<ToggleFriendTopRequest>,
    ) -> Result<Response<ToggleFriendTopResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id.clone();
        let friend_id = req.friend_id.clone();
        let is_top = req.is_top;
        
        // 更新置顶状态
        match self.toggle_friend_top_helper(&user_id, &friend_id, is_top).await {
            Ok(success) => {
                Ok(Response::new(ToggleFriendTopResponse {
                    success: success,
                }))
            },
            Err(e) => {
                error!("设置好友置顶状态失败: {:?}", e);
                Err(e)
            }
        }
    }

    // 更新好友备注
    async fn update_friend_remark(
        &self,
        request: Request<UpdateFriendRemarkRequest>,
    ) -> Result<Response<UpdateFriendRemarkResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;
        let friend_id = req.friend_id;
        let remark = req.remark;
        
        // 检查用户是否存在
        self.check_user_exists(&user_id).await?;
        
        // 检查好友关系
        match self.repository.check_friendship(&user_id, &friend_id).await {
            Ok(status) => {
                if status != Some(FriendshipStatus::Accepted) {
                    return Err(Status::failed_precondition("不是好友关系，无法更新备注"));
                }
            },
            Err(e) => {
                error!("检查好友关系失败: {}", e);
                return Err(Status::internal("内部服务错误"));
            }
        }
        
        // 更新好友备注
        match self.repository.update_friend_remark(&user_id, &friend_id, &remark).await {
            Ok(friend) => {
                // 转换为proto对象
                let proto_friend = friend.detailed_friend_to_proto();
                
                Ok(Response::new(UpdateFriendRemarkResponse {
                    success: true,
                    friend: Some(proto_friend),
                }))
            },
            Err(e) => {
                error!("更新好友备注失败: {}", e);
                Err(Status::internal(format!("更新好友备注失败: {}", e)))
            }
        }
    }

    // 获取用户黑名单列表
    async fn get_user_blacklist(
        &self,
        request: Request<GetUserBlacklistRequest>,
    ) -> Result<Response<GetUserBlacklistResponse>, Status> {
        let req = request.into_inner();
        
        let user_id = req.user_id.clone();
        
        // 检查用户是否存在
        self.check_user_exists(&user_id).await?;
        
        // 获取带用户信息的黑名单列表
        match self.repository.get_user_blacklist_with_info(&user_id, None, None).await {
            Ok(blacklist_with_info) => {
                let blacklist_protos = blacklist_with_info
                    .into_iter()
                    .map(|(blacklist, username, nickname, avatar_url)| {
                        UserBlacklistWithInfo {
                            blacklist: Some(blacklist.to_proto()),
                            username,
                            nickname,
                            avatar_url,
                        }
                    })
                    .collect();
                
                Ok(Response::new(GetUserBlacklistResponse {
                    blacklist: blacklist_protos,
                    total: 0, // 保留字段，但不再使用
                }))
            }
            Err(e) => {
                error!("获取用户黑名单列表失败: {}", e);
                Err(Status::internal("获取用户黑名单列表失败"))
            }
        }
    }

    // 是否拉黑状态
    async fn is_blocked(&self, request: Request<IsBlockedRequest>) -> Result<Response<IsBlockedResponse>, Status> {
        let req = request.into_inner();

        let user_id = req.user_id.clone();
        let blocked_user_id = req.blocked_user_id.clone();
         match self.repository.is_user_blocked(&user_id, &blocked_user_id).await {
            Ok(is_blocked) => {
                Ok(Response::new(IsBlockedResponse {
                    is_blocked,
                }))
            },
            Err(e) => {
                error!("获取是否拉黑状态失败: {}", e);
                Err(Status::internal("获取是否拉黑状态失败"))
            }
        }
        
    }
}
