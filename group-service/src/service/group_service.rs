use common::proto::group::{
    CreateGroupRequest, GetGroupRequest, UpdateGroupRequest, DeleteGroupRequest,
    AddMemberRequest, RemoveMemberRequest, UpdateMemberRoleRequest,
    GetMembersRequest, GetUserGroupsRequest, CheckMembershipRequest,
    DeleteGroupResponse, MemberResponse, GetMembersResponse, GetUserGroupsResponse,
    CheckMembershipResponse, GroupResponse, RemoveMemberResponse, MemberRole,
};
use common::proto::group::group_service_server::GroupService;
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use uuid::Uuid;
use tracing::{info, error};

use crate::repository::group_repository::GroupRepository;
use crate::repository::member_repository::MemberRepository;

pub struct GroupServiceImpl {
    group_repository: GroupRepository,
    member_repository: MemberRepository,
}

impl GroupServiceImpl {
    pub fn new(pool: PgPool) -> Self {
        Self {
            group_repository: GroupRepository::new(pool.clone()),
            member_repository: MemberRepository::new(pool),
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
        
        let owner_id = req.owner_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        match self.group_repository.create_group(
            req.name, 
            req.description, 
            req.avatar_url, 
            owner_id
        ).await {
            Ok(group) => {
                // 将创建者添加为群主
                match self.member_repository.add_member(
                    group.id,
                    owner_id,
                    "PLACEHOLDER".to_string(), // 实际应用中应该从user-service获取
                    None,
                    None,
                    MemberRole::Owner,
                ).await {
                    Ok(_) => {
                        let member_count = 1; // 刚创建时只有群主一人
                        info!("创建群组成功: {:?}", group);
                        Ok(Response::new(GroupResponse {
                            group: Some(group.to_proto(member_count)),
                        }))
                    }
                    Err(e) => {
                        error!("添加群主失败: {}", e);
                        Err(Status::internal("创建群组后添加群主失败"))
                    }
                }
            }
            Err(e) => {
                error!("创建群组失败: {}", e);
                Err(Status::internal("创建群组失败"))
            }
        }
    }
    
    // 获取群组信息
    async fn get_group(
        &self,
        request: Request<GetGroupRequest>,
    ) -> Result<Response<GroupResponse>, Status> {
        let req = request.into_inner();
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        match self.group_repository.get_group(group_id).await {
            Ok(group) => {
                // 获取成员数量
                let member_count = match self.group_repository.get_member_count(group_id).await {
                    Ok(count) => count,
                    Err(_) => 0,
                };
                
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
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        match self.group_repository.update_group(
            group_id,
            req.name,
            req.description,
            req.avatar_url,
        ).await {
            Ok(group) => {
                // 获取成员数量
                let member_count = match self.group_repository.get_member_count(group_id).await {
                    Ok(count) => count,
                    Err(_) => 0,
                };
                
                info!("更新群组信息成功: {:?}", group);
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
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        match self.group_repository.delete_group(group_id, user_id).await {
            Ok(success) => {
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
    ) -> Result<Response<MemberResponse>, Status> {
        let req = request.into_inner();
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let added_by_id = req.added_by_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的操作者ID: {}", e)))?;
        
        // 检查添加者权限
        match self.member_repository.get_member_role(group_id, added_by_id).await {
            Ok(role) => {
                if role < MemberRole::Admin as i32 {
                    return Err(Status::permission_denied("没有添加成员的权限"));
                }
            }
            Err(_) => {
                return Err(Status::permission_denied("操作者不是群组成员"));
            }
        }
        
        // 检查用户是否已经是成员
        match self.member_repository.check_membership(group_id, user_id).await {
            Ok((is_member, _)) => {
                if is_member {
                    return Err(Status::already_exists("用户已经是群组成员"));
                }
            }
            Err(e) => {
                error!("检查成员资格失败: {}", e);
                return Err(Status::internal("检查成员资格失败"));
            }
        }
        
        // 添加成员
        match self.member_repository.add_member(
            group_id,
            user_id,
            "PLACEHOLDER".to_string(), // 实际应用中应该从user-service获取
            None,
            None,
            req.role(),
        ).await {
            Ok(member) => {
                info!("添加群组成员成功: {:?}", member);
                Ok(Response::new(MemberResponse {
                    member: Some(member.to_proto()),
                }))
            }
            Err(e) => {
                error!("添加群组成员失败: {}", e);
                Err(Status::internal("添加群组成员失败"))
            }
        }
    }
    
    // 移除群组成员
    async fn remove_member(
        &self,
        request: Request<RemoveMemberRequest>,
    ) -> Result<Response<RemoveMemberResponse>, Status> {
        let req = request.into_inner();
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let removed_by_id = req.removed_by_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的操作者ID: {}", e)))?;
        
        match self.member_repository.remove_member(group_id, user_id, removed_by_id).await {
            Ok(success) => {
                if success {
                    info!("移除群组成员成功: group_id={}, user_id={}", group_id, user_id);
                    Ok(Response::new(RemoveMemberResponse { success }))
                } else {
                    Err(Status::not_found("用户不是群组成员"))
                }
            }
            Err(e) => {
                error!("移除群组成员失败: {}", e);
                if e.to_string().contains("没有权限") {
                    Err(Status::permission_denied(e.to_string()))
                } else if e.to_string().contains("无法移除") {
                    Err(Status::permission_denied(e.to_string()))
                } else {
                    Err(Status::internal("移除群组成员失败"))
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
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        let updated_by_id = req.updated_by_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的操作者ID: {}", e)))?;
        
        match self.member_repository.update_member_role(group_id, user_id, updated_by_id, req.role()).await {
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
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        match self.member_repository.get_members(group_id).await {
            Ok(members) => {
                let proto_members = members.into_iter()
                    .map(|m| m.to_proto())
                    .collect();
                
                Ok(Response::new(GetMembersResponse {
                    members: proto_members,
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
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        match self.group_repository.get_user_groups(user_id).await {
            Ok(groups) => {
                let proto_groups = groups.into_iter()
                    .map(|g| g.to_proto())
                    .collect();
                
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
        
        let group_id = req.group_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的群组ID: {}", e)))?;
        
        let user_id = req.user_id.parse::<Uuid>()
            .map_err(|e| Status::invalid_argument(format!("无效的用户ID: {}", e)))?;
        
        match self.member_repository.check_membership(group_id, user_id).await {
            Ok((is_member, role)) => {
                Ok(Response::new(CheckMembershipResponse {
                    is_member,
                    role: if is_member {
                        role.map(|r| r.into())
                    } else {
                        None
                    },
                }))
            }
            Err(e) => {
                error!("检查成员资格失败: {}", e);
                Err(Status::internal("检查成员资格失败"))
            }
        }
    }
}