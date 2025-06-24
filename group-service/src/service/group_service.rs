use std::sync::Arc;
use chrono::Utc;
use crate::repository::group_announcements_repository::GroupAnnouncementRepository;
use crate::repository::group_blacklist_repository::GroupBlacklistRepository;
use crate::repository::group_mutes_repository::GroupMutesRepository;
use crate::repository::group_repository::GroupRepository;
use crate::repository::group_settings_repository::GroupSettingsRepository;
use crate::repository::member_repository::MemberRepository;
use crate::repository::member_settings_repository::MemberSettingsRepository;
use common::config::ConfigLoader;
use common::grpc_client::base::get_rpc_client;
use common::proto::group::group_service_server::GroupService;
use common::proto::group::{AddMemberRequest, AddMemberResponse, AddToBlacklistRequest, AnnouncementResponse, BlacklistResponse, CheckMembershipRequest, CheckMembershipResponse, CheckUserMuteStatusRequest, CheckUserMuteStatusResponse, CreateAnnouncementRequest, CreateGroupQrcodeRequest, CreateGroupRequest, DeleteAnnouncementRequest, DeleteAnnouncementResponse, DeleteGroupRequest, DeleteGroupResponse, GetAnnouncementRequest, GetBlacklistRequest, GetBlacklistResponse, GetGroupAnnouncementsRequest, GetGroupAnnouncementsResponse, GetGroupQrcodeRequest, GetGroupRequest, GetGroupSettingsRequest, GetMemberSettingsRequest, GetMembersRequest, GetMembersResponse, GetMutedMembersRequest, GetMutedMembersResponse, GetUserGroupsRequest, GetUserGroupsResponse, GroupQrcodeResponse, GroupResponse, GroupSettingsResponse, LeaveGroupRequest, LeaveGroupResponse, MemberResponse, MemberRole, MemberSettingsResponse, MuteMemberRequest, MuteResponse, RemoveFromBlacklistRequest, RemoveFromBlacklistResponse, RemoveMemberRequest, RemoveMemberResponse, SearchUserGroupsRequest, SearchUserGroupsResponse, UnmuteMemberRequest, UnmuteResponse, UpdateGroupRequest, UpdateGroupSettingsRequest, UpdateMemberRoleRequest, UpdateMemberSettingsRequest};
use common::proto::user::user_service_client::UserServiceClient;
use common::service_discovery::LbWithServiceDiscovery;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use tracing::{error, info, debug};
use cache::Cache;
use common::proto::message::chat_service_client::ChatServiceClient;
use common::proto::message::{ContentType, Msg, MsgType, PlatformType, SendMsgRequest};
use common::proto::message::msg_service_client::MsgServiceClient;
use msg_storage::{MsgStoreRepo, MsgRecBoxRepo};
use crate::model::group::Group;
use serde_json;

pub struct GroupServiceImpl {
    group_repository: GroupRepository,
    member_repository: MemberRepository,
    announcement_repository: GroupAnnouncementRepository,
    settings_repository: GroupSettingsRepository,
    blacklist_repository: GroupBlacklistRepository,
    // mutes_repository: GroupMutesRepository,
    member_settings_repository: MemberSettingsRepository,
    user_service_client: UserServiceClient<LbWithServiceDiscovery>,
    chat_service_client: ChatServiceClient<LbWithServiceDiscovery>,
    msg_rec_box: Arc<dyn MsgRecBoxRepo>,
    cache: Arc<dyn Cache>,
}

impl GroupServiceImpl {
    pub async fn new(pool: PgPool) -> anyhow::Result<Self> {
        let config = ConfigLoader::get_global().expect("Failed to get global config");
        let user_service_client = get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(&config, "user".to_string()).await?;
        let chat_service_client = get_rpc_client::<ChatServiceClient<LbWithServiceDiscovery>>(&config, "chat".to_string()).await?;
        
        // 初始化消息存储
        let msg_rec_box = msg_storage::msg_rec_box_repo(&config).await?;
        
        // 初始化Redis缓存连接
        // 用于缓存用户状态和序列号信息
        let cache = cache::cache(&config).await;
        Ok(Self {
            group_repository: GroupRepository::new(pool.clone()),
            member_repository: MemberRepository::new(pool.clone()),
            announcement_repository: GroupAnnouncementRepository::new(pool.clone()),
            settings_repository: GroupSettingsRepository::new(pool.clone()),
            blacklist_repository: GroupBlacklistRepository::new(pool.clone()),
            // mutes_repository: GroupMutesRepository::new(pool.clone()),
            member_settings_repository: MemberSettingsRepository::new(pool.clone()),
            user_service_client,
            chat_service_client,
            msg_rec_box,
            cache: cache
        })
    }

    // 批量获取用户信息的辅助方法
    async fn fetch_users_info(&self, user_ids: Vec<String>) -> Result<std::collections::HashMap<String, common::proto::user::User>, Status> {
        if user_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        // 使用GetUsersByIds批量获取用户信息
        let request = tonic::Request::new(common::proto::user::GetUsersByIdsRequest {
            user_ids: user_ids.clone(),
        });

        match self.user_service_client.clone().get_users_by_ids(request).await {
            Ok(response) => {
                let users = response.into_inner().users;
                // 构建用户ID到用户信息的映射
                let mut user_map = std::collections::HashMap::new();
                for user in users {
                    user_map.insert(user.id.clone(), user);
                }
                Ok(user_map)
            }
            Err(e) => {
                error!("获取用户信息失败: {}", e);
                // 返回空映射而不是错误，避免因获取用户信息失败而中断主流程
                Ok(std::collections::HashMap::new())
            }
        }
    }
    
    // 创建默认的成员设置
    async fn create_default_member_settings(&self, group_id: &str, user_id: &str, user_info: Option<&common::proto::user::User>) -> Result<(), Status> {
        // 获取用户昵称，如果没有则使用空字符串
        let nickname_in_group = match user_info {
            Some(user) => user.nickname.clone().unwrap_or_default(),
            None => String::new(),
        };
        
        // 创建默认设置（不开启通知静音）
        match self.member_settings_repository.update_member_settings(
            group_id.to_string(),
            user_id.to_string(),
            Some(false), // 默认不静音通知
            Some(nickname_in_group),
            Some(String::new()), // 默认空备注
            Some(false),         // 默认不置顶
            Some(false),         // 默认不开启撤回通知
            Some(true),          // 默认显示群昵称
        ).await {
            Ok(_) => {
                debug!("已为用户 {} 在群组 {} 创建默认设置", user_id, group_id);
                Ok(())
            }
            Err(e) => {
                error!("创建成员默认设置失败: {}", e);
                // 返回成功而不是错误，避免因设置失败而中断主流程
                Ok(())
            }
        }
    }
    

    
    // 创建群聊消息
    async fn send_create_group_message(&self, owner_id: &str, group_id: &str, message: &str) {
        // 创建好友申请请求消息
        let create_group_msg = Msg {
            send_id: owner_id.to_string(),
            msg_type: MsgType::GroupInviteNew as i32,
            content_type: ContentType::Text as i32,
            content: message.as_bytes().to_vec(),
            group_id: group_id.to_string(),
            create_time: Utc::now().timestamp_millis(),
            receiver_id: group_id.to_string(),
            ..Default::default()
        };
        // 创建SendMsgRequest
        let request = SendMsgRequest {
            message: Some(create_group_msg),
        };

        // 调用chat服务发送消息
        let mut chat_client = self.chat_service_client.clone();
        match chat_client.send_msg(request).await {
            Ok(response) => {
                info!("创建群聊提示消息发送成功: {:?}", response.into_inner());
            }
            Err(e) => {
                error!("创建群聊提示消息发送失败: {:?}", e);
            }
        }
    }

    // 发送群解散消息通知
    async fn send_group_dismiss_message(&self, dismisser_id: &str, group_id: &str) {
        // 创建群解散消息
        let dismiss_msg = Msg {
            send_id: dismisser_id.to_string(),
            receiver_id: group_id.to_string(),
            msg_type: MsgType::GroupDismiss as i32,
            content_type: ContentType::Text as i32,
            content: "群组已被解散".to_string().as_bytes().to_vec(),
            group_id: group_id.to_string(),
            create_time: Utc::now().timestamp_millis(),
            ..Default::default()
        };
        
        // 创建SendMsgRequest
        let request = SendMsgRequest {
            message: Some(dismiss_msg),
        };

        // 调用chat服务发送消息
        let mut chat_client = self.chat_service_client.clone();
        match chat_client.send_msg(request).await {
            Ok(response) => {
                debug!("群解散消息响应: {:?}", response.into_inner());
            }
            Err(e) => {
                error!("群解散消息发送失败: group_id={}, error={:?}", group_id, e);
            }
        }
    }

    // 发送群组更新消息通知
    async fn send_group_update_message(&self, group: Group) {
        
        // 创建群组更新消息
        let update_msg = Msg {
            send_id: group.owner_id.clone(),
            receiver_id: group.id.clone(),
            msg_type: MsgType::GroupUpdate as i32,
            content_type: ContentType::Text as i32,
            content: "群头像，昵称，群公告，xxx，等发生变化".to_string().as_bytes().to_vec(),
            group_id: group.id.clone(),
            create_time: Utc::now().timestamp_millis(),
            ..Default::default()
        };
        
        // 创建SendMsgRequest
        let request = SendMsgRequest {
            message: Some(update_msg),
        };

        // 调用chat服务发送消息
        let mut chat_client = self.chat_service_client.clone();
        match chat_client.send_msg(request).await {
            Ok(response) => {
                debug!("群组更新消息发送成功: {:?}", response.into_inner());
            }
            Err(e) => {
                error!("群组更新消息发送失败: group_id={}, error={:?}", group.id, e);
            }
        }
    }

    /// 添加群组成员的辅助方法
    /// 
    /// 统一处理添加成员和创建默认设置的逻辑
    async fn add_group_member(
        &self,
        group_id: &str,
        user_id: &str,
        role: MemberRole,
        user_info_map: &std::collections::HashMap<String, common::proto::user::User>,
    ) -> Result<crate::model::member::Member, Status> {
        // 添加成员到群组
        let member = self
            .member_repository
            .add_member(
                group_id.to_string(),
                user_id.to_string(),
                None,
                None,
                None,
                role,
            )
            .await
            .map_err(|e| {
                error!("添加成员到群组失败: {}", e);
                Status::internal("添加成员失败")
            })?;

        // 为成员创建默认设置
        self.create_default_member_settings(
            group_id,
            user_id,
            user_info_map.get(user_id),
        )
        .await?;

        Ok(member)
    }

    // 发送添加群组成员消息通知
    async fn send_add_member_message(&self, added_by_id: &str, group_id: &str, message: &str) {
        // 创建添加成员消息
        let add_member_msg = Msg {
            send_id: added_by_id.to_string(),
            msg_type: MsgType::GroupInviteNew as i32,  // 使用邀请新成员消息类型
            content_type: ContentType::Text as i32,
            content: message.as_bytes().to_vec(),
            group_id: group_id.to_string(),
            create_time: Utc::now().timestamp_millis(),
            receiver_id: group_id.to_string(),
            ..Default::default()
        };

        // 创建SendMsgRequest并发送消息
        let request = SendMsgRequest {
            message: Some(add_member_msg),
        };

        let mut chat_client = self.chat_service_client.clone();
        match chat_client.send_msg(request).await {
            Ok(response) => {
                info!("添加群组成员消息发送成功: {:?}", response.into_inner());
            }
            Err(e) => {
                error!("添加群组成员消息发送失败: group_id={}, error={:?}", group_id, e);
            }
        }
    }

    // 发送移除群组成员消息通知
    async fn send_remove_member_message(&self, removed_by_id: &str, group_id: &str, message: &str) {
        // 创建移除成员消息
        let remove_member_msg = Msg {
            send_id: removed_by_id.to_string(),
            msg_type: MsgType::GroupMemberExit as i32,  // 使用退出群成员消息类型
            content_type: ContentType::Text as i32,
            content: message.as_bytes().to_vec(),
            group_id: group_id.to_string(),
            create_time: Utc::now().timestamp_millis(),
            receiver_id: group_id.to_string(),
            ..Default::default()
        };

        // 创建SendMsgRequest并发送消息
        let request = SendMsgRequest {
            message: Some(remove_member_msg),
        };

        let mut chat_client = self.chat_service_client.clone();
        match chat_client.send_msg(request).await {
            Ok(response) => {
                info!("退出群组成员消息发送成功: {:?}", response.into_inner());
            }
            Err(e) => {
                error!("退出群组成员消息发送失败: group_id={}, error={:?}", group_id, e);
            }
        }
    }
}

#[tonic::async_trait]
impl GroupService for GroupServiceImpl {
    // 创建群组
    async fn create_group(
        &self,
        request: Request<CreateGroupRequest>,
    ) -> Result<Response<GroupResponse>, Status> {
        let req = request.into_inner();
        let owner_id = req.owner_id.clone();

        // 创建群组
        let group = match self
            .group_repository
            .create_group(req.name.clone(), req.description, req.avatar_url, owner_id.clone())
            .await
        {
            Ok(group) => group,
            Err(e) => {
                error!("创建群组失败: {}", e);
                return Err(Status::internal("创建群组失败"));
            }
        };

        // 收集所有需要添加的成员ID（包括群主）
        let mut all_member_ids = vec![owner_id.clone()];
        all_member_ids.extend(req.members.clone());
        
        // 批量获取用户信息
        let user_info_map = self.fetch_users_info(all_member_ids.clone()).await?;

        // 批量添加成员
        let mut members = Vec::new();
        let mut success_count = 0;

        for user_id in all_member_ids.clone() {
            let role = if user_id == owner_id {
                MemberRole::Owner
            } else {
                MemberRole::Member
            };

            match self.add_group_member(&group.id, &user_id, role, &user_info_map).await {
                Ok(member) => {
                    members.push(member);
                    success_count += 1;
                }
                Err(e) => {
                    error!("添加成员 {} 失败: {}", user_id, e);
                    // 如果是群主添加失败，则整个创建失败
                    if user_id == owner_id {
                        return Err(Status::internal("创建群组后添加群主失败"));
                    }
                }
            }
        }

        // 加入缓存
        if let Err(e) = self
            .cache
            .save_group_members_id(&group.id.clone(), all_member_ids.clone())
            .await
        {
            error!("保存群组成员ID到缓存失败: {:?}", e);
        }
        
        info!(
            "创建群组成功: {:?}, 成功添加成员数: {}",
            group, success_count
        );
        
        // 发送群组创建成功消息
        self.send_create_group_message(
            &owner_id, 
            &group.id, 
            &format!("{}群组创建成功", req.name)
        ).await;
        
        Ok(Response::new(GroupResponse {
            group: Some(group.to_proto(success_count)),
        }))
    }

    // 获取群组信息
    async fn get_group(
        &self,
        request: Request<GetGroupRequest>,
    ) -> Result<Response<GroupResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();

        match self.group_repository.get_group(group_id.clone()).await {
            Ok(group) => {
                // 获取成员数量
                let member_count = self.group_repository.get_member_count(group_id).await.unwrap_or_else(|_| 0);

                Ok(Response::new(GroupResponse {
                    group: Some(group.to_proto(member_count)),
                }))
            }
            Err(e) => {
                error!("获取群组信息失败: {}", e);
                Err(Status::not_found("群组不存在"))
            }
        }
    }

    // 更新群组信息
    async fn update_group(
        &self,
        request: Request<UpdateGroupRequest>,
    ) -> Result<Response<GroupResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();

        match self
            .group_repository
            .update_group(group_id.clone(), req.name, req.description, req.avatar_url)
            .await
        {
            Ok(group) => {
                // 获取成员数量
                let member_count = self.group_repository.get_member_count(group_id).await.unwrap_or_else(|_| 0);

                info!("更新群组信息成功: {:?}", group);
                // 使用群主ID作为发送者
                self.send_group_update_message(group.clone()).await;
                Ok(Response::new(GroupResponse {
                    group: Some(group.to_proto(member_count)),
                }))
            }
            Err(e) => {
                error!("更新群组信息失败: {}", e);
                Err(Status::internal("更新群组信息失败"))
            }
        }
    }

    // 删除群组
    async fn delete_group(
        &self,
        request: Request<DeleteGroupRequest>,
    ) -> Result<Response<DeleteGroupResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();

        // 获取群组信息，同时验证用户权限
        match self.group_repository.get_group(group_id.clone()).await {
            Ok(group) => {
                // 直接通过owner_id判断是否为群主
                if group.owner_id != user_id {
                    return Err(Status::permission_denied("只有群主可以解散群组"));
                }
                self.send_group_dismiss_message(&user_id, &group_id).await;
                // 删除群组消息记录
                info!("开始删除群组消息记录: {}", group_id);
                // 删除MongoDB中的群组消息
                match self.msg_rec_box.delete_group_messages(&group_id).await {
                    Ok(deleted_count) => {
                        info!("成功删除MongoDB中的群组消息: {} 条", deleted_count);
                    }
                    Err(e) => {
                        error!("删除MongoDB中的群组消息失败: {}", e);
                    }
                }
                //休眠0.05秒
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => {
                error!("获取群组信息失败: {}", e);
                return Err(Status::not_found("群组不存在"));
            }
        };

        match self.group_repository.delete_group(group_id.clone(), user_id.clone()).await {
            Ok(success) => {
                // 删除群成员
                if let Err(e) = self.member_repository.delete_group_members(group_id.clone(), user_id.clone()).await {
                    error!("删除群成员失败: {}", e);
                    // 即使删除成员失败，也继续删除群组，因为群组已经被标记为删除
                }
                
                //删除缓存
                if let Err(e) = self.cache.del_group_members(&group_id).await {
                    error!("删除群组缓存失败: {}", e);
                }

                if success {
                    info!("删除群组成功: {}", group_id);
                    Ok(Response::new(DeleteGroupResponse { success }))
                } else {
                    Err(Status::not_found("群组不存在"))
                }
            }
            Err(e) => {
                error!("删除群组失败: {}", e);
                if e.to_string().contains("只有群主") {
                    Err(Status::permission_denied("只有群主可以删除群组"))
                } else {
                    Err(Status::internal("删除群组失败"))
                }
            }
        }
    }

    // 添加群组成员
    async fn add_member(
        &self,
        request: Request<AddMemberRequest>,
    ) -> Result<Response<AddMemberResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_ids = req.user_ids.clone();
        let added_by_id = req.added_by_id.clone();
        
        // 确保至少有一个要添加的成员
        if user_ids.is_empty() {
            return Err(Status::invalid_argument("成员列表不能为空"));
        }

        // 检查添加者权限
        match self
            .member_repository
            .get_member_role(group_id.clone(), added_by_id.clone())
            .await
        {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("没有添加成员的权限"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("操作者不是群组成员"));
            }
        }

        // 过滤掉已经是成员的用户ID
        let mut valid_user_ids = Vec::new();
        for user_id in user_ids.clone() {
            match self
                .member_repository
                .check_membership(group_id.clone(), user_id.clone())
                .await
            {
                Ok((is_member, _)) => {
                    if !is_member {
                        valid_user_ids.push(user_id);
                    } else {
                        info!("用户 {} 已经是群组 {} 的成员，跳过", user_id, group_id);
                    }
                }
                Err(e) => {
                    error!("检查成员资格失败: {} (user_id={})", e, user_id);
                    // 跳过错误，继续处理下一个用户
                }
            }
        }

        // 如果没有有效的用户ID，直接返回
        if valid_user_ids.is_empty() {
            return Ok(Response::new(AddMemberResponse { 
                success: true,
                added_count: 0
            }));
        }

        // 批量获取用户信息
        let users_map = self.fetch_users_info(valid_user_ids.clone()).await?;
        
        // 记录成功添加的成员数量
        let mut added_count = 0;
        
        // 逐个添加成员
        for user_id in valid_user_ids {
            // 从用户映射中获取用户信息
            let user_info = users_map.get(&user_id);
    
            // 添加成员
            match self
                .member_repository
                .add_member(
                    group_id.clone(),
                    user_id.clone(),
                    user_info.map(|u| u.username.clone()),
                    user_info.and_then(|u| u.nickname.clone()),
                    user_info.and_then(|u| u.avatar_url.clone()),
                    req.role(),
                )
                .await
            {
                Ok(_) => {
                    info!("添加群组成员成功: user_id={}", user_id);
                    
                    // 为新成员创建默认设置
                    if let Err(e) = self.create_default_member_settings(
                        &group_id, 
                        &user_id,
                        user_info
                    ).await {
                        error!("创建成员默认设置失败: {}", e);
                    }
                    
                    // 增加成功添加计数
                    added_count += 1;
                }
                Err(e) => {
                    error!("添加群组成员失败: {}", e);
                    // 继续添加其他成员
                }
            }
        }
        if added_count > 0 {
            //删除缓存
            self.cache.del_group_members(&group_id).await?;
            
            // 发送添加成员消息通知
            let message = if added_count == 1 {
                "新成员加入群聊".to_string()
            } else {
                format!("{} 位新成员加入群聊", added_count)
            };
            self.send_add_member_message(&added_by_id, &group_id, &message).await;
        }
        
        // 如果至少有一个成员添加成功，返回成功
        Ok(Response::new(AddMemberResponse { 
            success: added_count > 0,
            added_count
        }))
    }

    // 移除群组成员
    async fn remove_member(
        &self,
        request: Request<RemoveMemberRequest>,
    ) -> Result<Response<RemoveMemberResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_ids = req.user_ids.clone();
        let removed_by_id = req.removed_by_id.clone();
        
        // 确保至少有一个要移除的成员
        if user_ids.is_empty() {
            return Err(Status::invalid_argument("成员列表不能为空"));
        }

        // 移除成功的成员数量
        let mut success_count = 0;
        
        // 逐个移除成员
        for user_id in user_ids {
            match self
                .member_repository
                .remove_member(group_id.clone(), user_id.clone(), removed_by_id.clone())
                .await
            {
                Ok(success) => {
                    if success {
                        info!(
                            "移除群组成员成功: group_id={}, user_id={}",
                            group_id, user_id
                        );
                        success_count += 1;
                    } else {
                        info!(
                            "用户不是群组成员: group_id={}, user_id={}",
                            group_id, user_id
                        );
                    }
                }
                Err(e) => {
                    error!("移除群组成员失败: {} (user_id={})", e, user_id);
                    // 继续处理下一个成员
                }
            }
        }
        
        // 如果至少有一个成员被成功移除，返回成功
        if success_count > 0 {
            //删除缓存
            self.cache.del_group_members(&group_id).await?;
            
            // 发送移除成员消息通知
            let message = if success_count == 1 {
                "成员已退出群聊".to_string()
            } else {
                format!("{} 位成员已退出群聊", success_count)
            };
            self.send_remove_member_message(&removed_by_id, &group_id, &message).await;
            
            Ok(Response::new(RemoveMemberResponse { success: true }))
        } else {
            Err(Status::not_found("没有成功移除任何成员"))
        }
    }

    // 退出群组 (成员主动退出)
    async fn leave_group(
        &self,
        request: Request<LeaveGroupRequest>,
    ) -> Result<Response<LeaveGroupResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();

        // 调用member_repository的leave_group方法
        match self.member_repository.leave_group(group_id.clone(), user_id.clone()).await {
            Ok(success) => {
                if success {
                    info!("用户退出群组成功: group_id={}, user_id={}", group_id, user_id);

                    // 删除缓存
                    if let Err(e) = self.cache.del_group_members(&group_id).await {
                        error!("删除群组缓存失败: {}", e);
                    }

                    // 发送退出群组消息通知
                    self.send_remove_member_message(&user_id, &group_id, "成员已退出群聊").await;

                    Ok(Response::new(LeaveGroupResponse { success: true }))
                } else {
                    Err(Status::internal("退出群组失败"))
                }
            }
            Err(e) => {
                error!("退出群组失败: {}", e);
                if e.to_string().contains("群主不能直接退出群组") {
                    Err(Status::permission_denied("群主不能直接退出群组，请先转让群主身份"))
                } else if e.to_string().contains("用户不是群组成员") {
                    Err(Status::not_found("用户不是群组成员"))
                } else {
                    Err(Status::internal("退出群组失败"))
                }
            }
        }
    }


    // 更新成员角色
    async fn update_member_role(
        &self,
        request: Request<UpdateMemberRoleRequest>,
    ) -> Result<Response<MemberResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();
        let updated_by_id = req.updated_by_id.clone();
        let role = req.role();

        match self
            .member_repository
            .update_member_role(group_id, user_id, updated_by_id, role)
            .await
        {
            Ok(member) => {
                info!("更新成员角色成功: {:?}", member);
                Ok(Response::new(MemberResponse {
                    member: Some(member.to_proto()),
                }))
            }
            Err(e) => {
                error!("更新成员角色失败: {}", e);
                if e.to_string().contains("只有群主") {
                    Err(Status::permission_denied(e.to_string()))
                } else if e.to_string().contains("无法将成员提升") {
                    Err(Status::permission_denied(e.to_string()))
                } else {
                    Err(Status::internal("更新成员角色失败"))
                }
            }
        }
    }

    // 获取群组成员列表
    async fn get_members(
        &self,
        request: Request<GetMembersRequest>,
    ) -> Result<Response<GetMembersResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        
        // 解析可选参数
        let page = if req.page > 0 { Some(req.page) } else { None };
        let page_size = if req.page_size > 0 { Some(req.page_size) } else { None };

        match self.member_repository.get_members(group_id.clone(), page, page_size).await {
            Ok((members, total)) => {
                // 转换成员列表为 proto 对象
                let mut proto_members = Vec::with_capacity(members.len());


                // 批量获取群组中所有成员设置
                let member_setting_map = self.member_settings_repository.get_active_mutes_by_group_id(group_id.clone()).await.unwrap_or_else(|e| {
                    error!("批量获取群组禁言状态失败: {}", e);
                    std::collections::HashMap::new() // 出错时使用空映射继续处理
                });
                
                for member in members {
                    // 先创建基本的成员对象
                    let mut proto_member = member.to_proto();

                    
                    // 检查该成员是否在成员设置映射中
                    if let Some(member_setting) = member_setting_map.get(&member.user_id) {
                        // 设置成员群内昵称
                        proto_member.remark = member_setting.nickname_in_group.clone();
                    }
                    
                    proto_members.push(proto_member);
                }

                Ok(Response::new(GetMembersResponse {
                    members: proto_members,
                    total: total as i32,
                    page: page.unwrap_or(1),
                    page_size: page_size.unwrap_or(20),
                }))
            }
            Err(e) => {
                error!("获取群组成员列表失败: {}", e);
                Err(Status::internal("获取群组成员列表失败"))
            }
        }
    }

    // 获取用户加入的群组列表
    async fn get_user_groups(
        &self,
        request: Request<GetUserGroupsRequest>,
    ) -> Result<Response<GetUserGroupsResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;

        match self.group_repository.get_user_groups(user_id.clone()).await {
            Ok(groups) => {
                let proto_groups = groups.into_iter().map(|g| g.to_proto()).collect();

                Ok(Response::new(GetUserGroupsResponse {
                    groups: proto_groups,
                }))
            }
            Err(e) => {
                error!("获取用户群组列表失败: {}", e);
                Err(Status::internal("获取用户群组列表失败"))
            }
        }
    }

    // 检查用户是否在群组中
    async fn check_membership(
        &self,
        request: Request<CheckMembershipRequest>,
    ) -> Result<Response<CheckMembershipResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id;
        let user_id = req.user_id;

        match self
            .member_repository
            .check_membership(group_id, user_id)
            .await
        {
            Ok((is_member, role)) => Ok(Response::new(CheckMembershipResponse {
                is_member,
                role: if is_member {
                    role.map(|r| r.into())
                } else {
                    None
                },
            })),
            Err(e) => {
                error!("检查成员资格失败: {}", e);
                Err(Status::internal("检查成员资格失败"))
            }
        }
    }

    // 搜索用户加入的群组（按关键字）
    async fn search_user_groups(
        &self,
        request: Request<SearchUserGroupsRequest>,
    ) -> Result<Response<SearchUserGroupsResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;

         // 解析可选参数
         let page = (req.page > 0).then_some(req.page);
         let page_size = (req.page_size > 0).then_some(req.page_size);
         
         // 解析搜索关键词
         let keyword = (!req.keyword.is_empty()).then_some(req.keyword.clone());
 
        match self
            .group_repository
            .search_user_groups(
                user_id.clone(),
                keyword.as_deref(), 
                page, 
                page_size
            )
            .await
        {
            Ok((groups, total)) => {
                // 将群组信息转换为proto对象
                let proto_groups = groups.into_iter().map(|g| g.to_proto()).collect();

                Ok(Response::new(SearchUserGroupsResponse {
                    groups: proto_groups,
                    total: total as i32,
                    page: page.unwrap_or(1),
                    page_size: page_size.unwrap_or(10),
                }))
            }
            Err(e) => {
                error!("搜索用户群组失败: {}", e);
                Err(Status::internal("搜索用户群组失败"))
            }
        }
    }

    // 创建群公告
    async fn create_announcement(
        &self,
        request: Request<CreateAnnouncementRequest>,
    ) -> Result<Response<AnnouncementResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let creator_id = req.creator_id.clone();

        // 验证用户是否有权限创建公告（群主或管理员）
        match self.member_repository.get_member_role(group_id.clone(), creator_id.clone()).await {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("只有群主或管理员才能创建群公告"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("用户不是群组成员"));
            }
        }

        // 创建公告
        match self.announcement_repository.create_announcement(
            group_id,
            creator_id,
            if req.title.is_empty() { None } else { Some(req.title) },
            req.content,
            req.is_pinned,
        ).await {
            Ok(announcement) => {
                info!("创建群公告成功: {:?}", announcement);
                Ok(Response::new(AnnouncementResponse {
                    announcement: Some(announcement.to_proto()),
                }))
            }
            Err(e) => {
                error!("创建群公告失败: {}", e);
                Err(Status::internal("创建群公告失败"))
            }
        }
    }

    // 获取群公告
    async fn get_announcement(
        &self,
        request: Request<GetAnnouncementRequest>,
    ) -> Result<Response<AnnouncementResponse>, Status> {
        let req = request.into_inner();
        let announcement_id = req.announcement_id.clone();

        match self.announcement_repository.get_announcement(announcement_id).await {
            Ok(announcement) => {
                Ok(Response::new(AnnouncementResponse {
                    announcement: Some(announcement.to_proto()),
                }))
            }
            Err(e) => {
                error!("获取群公告失败: {}", e);
                Err(Status::not_found("公告不存在"))
            }
        }
    }

    // 获取群组所有公告
    async fn get_group_announcements(
        &self,
        request: Request<GetGroupAnnouncementsRequest>,
    ) -> Result<Response<GetGroupAnnouncementsResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();

        match self.announcement_repository.get_group_announcements(group_id).await {
            Ok(announcements) => {
                let proto_announcements = announcements
                    .into_iter()
                    .map(|a| a.to_proto())
                    .collect();

                Ok(Response::new(GetGroupAnnouncementsResponse {
                    announcements: proto_announcements,
                }))
            }
            Err(e) => {
                error!("获取群组公告列表失败: {}", e);
                Err(Status::internal("获取群组公告列表失败"))
            }
        }
    }

    // 删除群公告
    async fn delete_announcement(
        &self,
        request: Request<DeleteAnnouncementRequest>,
    ) -> Result<Response<DeleteAnnouncementResponse>, Status> {
        let req = request.into_inner();
        let announcement_id = req.announcement_id.clone();
        let deleted_by_id = req.deleted_by_id.clone();

        // 获取公告信息
        let announcement = match self.announcement_repository.get_announcement(announcement_id.clone()).await {
            Ok(a) => a,
            Err(_) => {
                return Err(Status::not_found("公告不存在"));
            }
        };

        // 验证用户权限（是创建者、群主或管理员）
        if deleted_by_id != announcement.creator_id {
            match self.member_repository.get_member_role(announcement.group_id.clone(), deleted_by_id.clone()).await {
                Ok(role) => {
                    if role < MemberRole::Admin as i32 {
                        return Err(Status::permission_denied("没有权限删除此公告"));
                    }
                }
                Err(_) => {
                    return Err(Status::permission_denied("用户不是群组成员"));
                }
            }
        }

        // 删除公告
        match self.announcement_repository.delete_announcement(announcement_id, deleted_by_id).await {
            Ok(success) => {
                if success {
                    info!("删除群公告成功");
                    Ok(Response::new(DeleteAnnouncementResponse { success }))
                } else {
                    Err(Status::not_found("公告不存在"))
                }
            }
            Err(e) => {
                error!("删除群公告失败: {}", e);
                Err(Status::internal("删除群公告失败"))
            }
        }
    }

    // 获取群组设置
    async fn get_group_settings(
        &self,
        request: Request<GetGroupSettingsRequest>,
    ) -> Result<Response<GroupSettingsResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();

        match self.settings_repository.get_group_settings(group_id).await {
            Ok(settings) => {
                Ok(Response::new(GroupSettingsResponse {
                    settings: Some(settings.to_proto()),
                }))
            }
            Err(e) => {
                error!("获取群组设置失败: {}", e);
                Err(Status::internal("获取群组设置失败"))
            }
        }
    }

    // 更新群组设置
    async fn update_group_settings(
        &self,
        request: Request<UpdateGroupSettingsRequest>,
    ) -> Result<Response<GroupSettingsResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let updated_by_id = req.updated_by_id.clone();

        // 验证更新者的权限 (群主或管理员)
        match self.member_repository.get_member_role(group_id.clone(), updated_by_id.clone()).await {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("只有群主或管理员才能更新群组设置"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("用户不是群组成员"));
            }
        }

        match self.settings_repository.update_group_settings(
            group_id,
            req.allow_member_friendship,
            req.join_approval_required,
            req.only_admin_can_invite,
            req.only_admin_can_modify,
            req.notify_member_join,
            req.all_member_muted,
        ).await {
            Ok(settings) => {
                info!("更新群组设置成功: {:?}", settings);
                Ok(Response::new(GroupSettingsResponse {
                    settings: Some(settings.to_proto()),
                }))
            }
            Err(e) => {
                error!("更新群组设置失败: {}", e);
                Err(Status::internal("更新群组设置失败"))
            }
        }
    }

    // 添加用户到黑名单
    async fn add_to_blacklist(
        &self,
        request: Request<AddToBlacklistRequest>,
    ) -> Result<Response<BlacklistResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();
        let creator_id = req.creator_id.clone();
        let reason = if req.reason.is_empty() { None } else { Some(req.reason) };

        // 验证操作者的权限 (群主或管理员)
        match self.member_repository.get_member_role(group_id.clone(), creator_id.clone()).await {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("只有群主或管理员才能添加黑名单"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("操作者不是群组成员"));
            }
        }

        // 检查目标用户的角色，不能将管理员或群主加入黑名单
        match self.member_repository.get_member_role(group_id.clone(), user_id.clone()).await {
            Ok(role) => {
                if role >= MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("不能将管理员或群主加入黑名单"));
                }
            }
            Err(_) => {
                // 用户不是群成员，可以加入黑名单
            }
        }

        match self.blacklist_repository.add_to_blacklist(group_id, user_id, creator_id, reason).await {
            Ok(entry) => {
                info!("添加用户到黑名单成功: {:?}", entry);
                Ok(Response::new(BlacklistResponse {
                    entry: Some(entry.to_proto()),
                }))
            }
            Err(e) => {
                error!("添加用户到黑名单失败: {}", e);
                if e.to_string().contains("已经在黑名单中") {
                    Err(Status::already_exists("该用户已经在黑名单中"))
                } else {
                    Err(Status::internal("添加用户到黑名单失败"))
                }
            }
        }
    }

    // 从黑名单中移除用户
    async fn remove_from_blacklist(
        &self,
        request: Request<RemoveFromBlacklistRequest>,
    ) -> Result<Response<RemoveFromBlacklistResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();
        let removed_by_id = req.removed_by_id.clone();

        // 验证操作者的权限 (群主或管理员)
        match self.member_repository.get_member_role(group_id.clone(), removed_by_id.clone()).await {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("只有群主或管理员才能移除黑名单"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("操作者不是群组成员"));
            }
        }

        match self.blacklist_repository.remove_from_blacklist(group_id, user_id).await {
            Ok(success) => {
                info!("从黑名单中移除用户成功");
                Ok(Response::new(RemoveFromBlacklistResponse { success }))
            }
            Err(e) => {
                error!("从黑名单中移除用户失败: {}", e);
                Err(Status::internal("从黑名单中移除用户失败"))
            }
        }
    }

    // 获取群组黑名单
    async fn get_blacklist(
        &self,
        request: Request<GetBlacklistRequest>,
    ) -> Result<Response<GetBlacklistResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();

        match self.blacklist_repository.get_blacklist(group_id).await {
            Ok(entries) => {
                let proto_entries = entries.into_iter().map(|e| e.to_proto()).collect();
                Ok(Response::new(GetBlacklistResponse { entries: proto_entries }))
            }
            Err(e) => {
                error!("获取群组黑名单失败: {}", e);
                Err(Status::internal("获取群组黑名单失败"))
            }
        }
    }

    // 禁言成员
    async fn mute_member(
        &self,
        request: Request<MuteMemberRequest>,
    ) -> Result<Response<MuteResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();
        let creator_id = req.creator_id.clone();

        // 检查目标用户的角色，不能禁言管理员或群主
        let creator_role = match self.member_repository.get_member_role(group_id.clone(), creator_id.clone()).await {
            Ok(role) => {
                // 验证操作者的权限 (群主或管理员)
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("只有群主或管理员才能禁言成员"));
                }
                role
            },
            Err(e) => {
                error!("操作者不是群组成员: {}", e);
                return Err(Status::permission_denied("操作者不是群组成员"));
            }
        };

        match self.member_repository.get_member_role(group_id.clone(), user_id.clone()).await {
            Ok(role) => {
                if role >= creator_role {
                    return Err(Status::permission_denied("不能禁言角色相同或更高的成员"));
                }
            }
            Err(_) => {
                return Err(Status::not_found("用户不是群组成员"));
            }
        }

        match self.member_repository.set_member_muted_status(group_id, user_id,1).await {
            Ok(success) => {
                info!("禁言成员成功");
                Ok(Response::new(MuteResponse { success }))
            }
            Err(e) => {
                error!("禁言成员失败: {}", e);
                Err(Status::internal("禁言成员失败"))
            }
        }
    }

    // 解除成员禁言
    async fn unmute_member(
        &self,
        request: Request<UnmuteMemberRequest>,
    ) -> Result<Response<UnmuteResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();
        let unmuted_by_id = req.unmuted_by_id.clone();

        // 验证操作者的权限 (群主或管理员)
        match self.member_repository.get_member_role(group_id.clone(), unmuted_by_id.clone()).await {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("只有群主或管理员才能解除禁言"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("操作者不是群组成员"));
            }
        }

        match self.member_repository.set_member_muted_status(group_id, user_id,0).await {
            Ok(success) => {
                info!("解除成员禁言成功");
                Ok(Response::new(UnmuteResponse { success }))
            }
            Err(e) => {
                error!("解除成员禁言失败: {}", e);
                Err(Status::internal("解除成员禁言失败"))
            }
        }
    }

    // 获取被禁言的成员列表
    async fn get_muted_members(
        &self,
        request: Request<GetMutedMembersRequest>,
    ) -> Result<Response<GetMutedMembersResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();

        match self.member_repository.get_muted_members(group_id).await {
            Ok(entries) => {
                let proto_entries = entries.into_iter().map(|e| e.to_proto()).collect();
                Ok(Response::new(GetMutedMembersResponse { entries: proto_entries }))
            }
            Err(e) => {
                error!("获取被禁言的成员列表失败: {}", e);
                Err(Status::internal("获取被禁言的成员列表失败"))
            }
        }
    }

    // 获取成员设置
    async fn get_member_settings(
        &self,
        request: Request<GetMemberSettingsRequest>,
    ) -> Result<Response<MemberSettingsResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();

        match self.member_settings_repository.get_member_settings(group_id, user_id).await {
            Ok(settings) => {
                Ok(Response::new(MemberSettingsResponse {
                    settings: Some(settings.to_proto()),
                }))
            }
            Err(e) => {
                error!("获取成员设置失败: {}", e);
                Err(Status::internal("获取成员设置失败"))
            }
        }
    }

    // 更新成员设置
    async fn update_member_settings(
        &self,
        request: Request<UpdateMemberSettingsRequest>,
    ) -> Result<Response<MemberSettingsResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id.clone();
        let user_id = req.user_id.clone();

        // 验证用户是否是群组成员
        match self.member_repository.check_membership(group_id.clone(), user_id.clone()).await {
            Ok((is_member, _)) => {
                if !is_member {
                    return Err(Status::permission_denied("用户不是群组成员"));
                }
            }
            Err(e) => {
                error!("验证用户是否是群组成员失败: {}", e);
                return Err(Status::internal("验证用户是否是群组成员失败"));
            }
        }

        match self.member_settings_repository.update_member_settings(
            group_id,
            user_id,
            req.mute_notifications,
            req.nickname_in_group,
            req.remark,
            req.is_top,
            req.recall_notification,
            req.show_nickname,
        ).await {
            Ok(settings) => {
                info!("更新成员设置成功: {:?}", settings);
                Ok(Response::new(MemberSettingsResponse {
                    settings: Some(settings.to_proto()),
                }))
            }
            Err(e) => {
                error!("更新成员设置失败: {}", e);
                Err(Status::internal("更新成员设置失败"))
            }
        }
    }

    // 创建群二维码
    async fn create_group_qrcode(
        &self,
        _request: Request<CreateGroupQrcodeRequest>,
    ) -> Result<Response<GroupQrcodeResponse>, Status> {
        // 由于尚未实现QRCode相关功能，返回未实现错误
        Err(Status::unimplemented("创建群二维码功能尚未实现"))
    }

    // 获取群二维码
    async fn get_group_qrcode(
        &self,
        _request: Request<GetGroupQrcodeRequest>,
    ) -> Result<Response<GroupQrcodeResponse>, Status> {
        // 由于尚未实现QRCode相关功能，返回未实现错误
        Err(Status::unimplemented("获取群二维码功能尚未实现"))
    }

    // 检查用户禁言状态
    async fn check_user_mute_status(
        &self,
        request: Request<CheckUserMuteStatusRequest>,
    ) -> Result<Response<CheckUserMuteStatusResponse>, Status> {
        let req = request.into_inner();
        let group_id = req.group_id;
        let user_id = req.user_id;

        // 先检查用户是否是群组成员
        match self.member_repository.check_membership(group_id.clone(), user_id.clone()).await {
            Ok((is_member, _)) => {
                if !is_member {
                    return Err(Status::not_found("用户不是群组成员"));
                }
            }
            Err(e) => {
                error!("检查成员资格失败: {}", e);
                return Err(Status::internal("检查成员资格失败"));
            }
        }
        match self.member_repository.check_user_mute_status(group_id.clone(), user_id.clone()).await { 
            Ok(mute_entry_opt) => {
                Ok(Response::new(CheckUserMuteStatusResponse {
                    is_muted: mute_entry_opt,
                }))
            }
            Err(e) => {
                error!("检查用户禁言状态失败: {}", e);
                Err(Status::internal("检查用户禁言状态失败"))
            }
        }
    }
}
