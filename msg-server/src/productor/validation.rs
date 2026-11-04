//! 消息发送权限校验
//!
//! 从 `productor.rs` 拆出的独立关注点：所有「能不能发」的判定都在这里，
//! 「怎么发」（Kafka 生产、会话查询）留在父模块。
//!
//! 校验策略（缓存优先，RPC 兜底，fail-open 可配置）：
//! - 好友/黑名单集合由 relation-service 在关系变更时同步维护，
//!   正常路径零 RPC；
//! - 校验链路故障时按 `validation.fail_open` 配置放行或拒绝。

use tonic::Status;
use tracing::{debug, error, warn};

use contract::proto::friend::IsBlockedRequest;
use contract::proto::group::{CheckMembershipRequest, GetGroupRequest};
use contract::proto::friend::{CheckFriendshipRequest, FriendRelationType};
use contract::proto::message::{Msg, MsgType};

use super::ChatRpcService;

impl ChatRpcService {
    /// 检查好友关系和拉黑状态
    ///
    /// 两项检查相互独立，**并行发起**，总耗时约等于最慢一项。
    ///
    /// # 返回值
    /// * `Ok(true)` - 可以发送消息
    /// * `Ok(false)` - 不能发送消息（被拉黑或非好友）
    /// * `Err(Status)` - 校验链路故障且配置为 fail-closed
    pub(super) async fn check_friend_relation(
        &self,
        send_id: &str,
        receiver_id: &str,
    ) -> Result<bool, Status> {
        let (blacklist, friendship) = tokio::join!(
            self.check_blacklist_bidirectional(send_id, receiver_id),
            self.check_friendship_cached(send_id, receiver_id),
        );

        match (blacklist, friendship) {
            (Ok(result), Ok(is_friend)) => {
                if result.has_blacklist {
                    debug!("用户 {} 与 {} 存在黑名单关系，拒绝发送", send_id, receiver_id);
                    Ok(false)
                } else if is_friend {
                    debug!("用户 {} 和 {} 是好友关系", send_id, receiver_id);
                    Ok(true)
                } else {
                    debug!("用户 {} 和 {} 不是好友关系", send_id, receiver_id);
                    Ok(false)
                }
            }
            (Err(reason), _) | (_, Err(reason)) => {
                if self.fail_open {
                    warn!("权限校验链路故障（{}），fail-open 放行: {} -> {}", reason, send_id, receiver_id);
                    Ok(true)
                } else {
                    error!("权限校验链路故障（{}），fail-closed 拒绝: {} -> {}", reason, send_id, receiver_id);
                    Err(Status::internal("权限校验服务暂时不可用，请稍后重试"))
                }
            }
        }
    }

    /// 双向拉黑检查（缓存优先，RPC并行兜底）
    async fn check_blacklist_bidirectional(
        &self,
        send_id: &str,
        receiver_id: &str,
    ) -> Result<cache::BlacklistCheckResult, String> {
        match self
            .relation
            .check_bidirectional_blacklist(send_id, receiver_id)
            .await
        {
            Ok(result) => Ok(result),
            Err(e) => {
                debug!("黑名单缓存查询失败，降级RPC: {}", e);
                let mut client_a = self.friend_client.clone();
                let mut client_b = self.friend_client.clone();
                let (blocked_by_receiver, blocked_by_sender) = tokio::join!(
                    client_a.is_blocked(IsBlockedRequest {
                        user_id: receiver_id.to_string(),
                        blocked_user_id: send_id.to_string(),
                    }),
                    client_b.is_blocked(IsBlockedRequest {
                        user_id: send_id.to_string(),
                        blocked_user_id: receiver_id.to_string(),
                    }),
                );
                match (blocked_by_receiver, blocked_by_sender) {
                    (Ok(a), Ok(b)) => Ok(cache::BlacklistCheckResult::new(
                        a.into_inner().is_blocked,
                        b.into_inner().is_blocked,
                    )),
                    (Err(e), _) | (_, Err(e)) => Err(format!("拉黑状态RPC检查失败: {}", e)),
                }
            }
        }
    }

    /// 好友关系检查（缓存优先，未命中走RPC确认）
    async fn check_friendship_cached(&self, send_id: &str, receiver_id: &str) -> Result<bool, String> {
        match self.relation.check_friendship_exists(send_id, receiver_id).await {
            Ok(true) => Ok(true),
            Ok(false) => {
                // 集合里没有 ≠ 一定不是好友（可能未预热），RPC确认
                debug!("好友关系缓存未命中，RPC确认: {} -> {}", send_id, receiver_id);
                self.rpc_check_friendship(send_id, receiver_id).await
            }
            Err(e) => {
                debug!("好友关系缓存查询失败，降级RPC: {}", e);
                self.rpc_check_friendship(send_id, receiver_id).await
            }
        }
    }

    /// RPC确认好友关系
    async fn rpc_check_friendship(&self, send_id: &str, receiver_id: &str) -> Result<bool, String> {
        let mut friend_client = self.friend_client.clone();
        match friend_client
            .check_friendship(CheckFriendshipRequest {
                user_id: send_id.to_string(),
                friend_id: receiver_id.to_string(),
            })
            .await
        {
            Ok(response) => {
                let status = response.into_inner().status;
                Ok(status == FriendRelationType::IsFriend as i32)
            }
            Err(e) => Err(format!("好友关系RPC检查失败: {}", e)),
        }
    }

    /// 检查群组成员身份
    ///
    /// # 返回值
    /// * `Ok(true)` - 用户是群组成员
    /// * `Ok(false)` - 用户不是群组成员
    /// * `Err(Status)` - 检查失败
    async fn check_group_membership(&self, user_id: &str, group_id: &str) -> Result<bool, Status> {
        let mut group_client = self.group_client.clone();
        match group_client.check_membership(CheckMembershipRequest {
            group_id: group_id.to_string(),
            user_id: user_id.to_string(),
        }).await {
            Ok(response) => {
                let membership = response.into_inner();
                Ok(membership.is_member)
            }
            Err(e) => {
                error!("检查群组成员身份失败: {}", e);
                Err(Status::internal("检查群组成员身份失败"))
            }
        }
    }

    /// 检查拉黑状态
    ///
    /// 缓存优先（正常路径零 RPC），缓存异常时降级 RPC，
    /// 双层故障时按 `validation.fail_open` 配置放行或拒绝。
    ///
    /// # 返回值
    /// * `Ok(true)` - 被拉黑
    /// * `Ok(false)` - 未被拉黑
    /// * `Err(Status)` - 校验链路故障且配置为 fail-closed
    async fn check_blocked_status(&self, user_id: &str, target_user_id: &str) -> Result<bool, Status> {
        match self.relation.is_user_in_blacklist(user_id, target_user_id).await {
            Ok(is_blocked) => {
                if is_blocked {
                    debug!("用户 {} 已被 {} 拉黑（缓存命中）", target_user_id, user_id);
                }
                Ok(is_blocked)
            }
            Err(e) => {
                debug!("黑名单缓存查询失败，降级RPC: {}", e);
                let mut friend_client = self.friend_client.clone();
                match friend_client.is_blocked(IsBlockedRequest {
                    user_id: user_id.to_string(),
                    blocked_user_id: target_user_id.to_string(),
                }).await {
                    Ok(response) => {
                        let is_blocked = response.into_inner().is_blocked;
                        if is_blocked {
                            debug!("用户 {} 已被 {} 拉黑（RPC确认）", target_user_id, user_id);
                        }
                        Ok(is_blocked)
                    }
                    Err(rpc_err) => {
                        let reason = format!("缓存: {}，RPC: {}", e, rpc_err);
                        if self.fail_open {
                            warn!("拉黑状态校验故障（{}），fail-open 放行", reason);
                            Ok(false)
                        } else {
                            error!("拉黑状态校验故障（{}），fail-closed 拒绝", reason);
                            Err(Status::internal("权限校验服务暂时不可用，请稍后重试"))
                        }
                    }
                }
            }
        }
    }

    /// 根据消息类型校验发送权限
    ///
    /// # 返回值
    /// * `Ok(())` - 校验通过，可以发送
    /// * `Err(Status)` - 校验失败，包含错误信息
    pub(super) async fn validate_message_permissions(&self, msg: &Msg) -> Result<(), Status> {
        let msg_type = MsgType::try_from(msg.msg_type)
            .map_err(|_| Status::invalid_argument("无效的消息类型"))?;

        // 根据消息类型进行校验
        match msg_type {
            // 单聊消息需要校验好友关系
            MsgType::SingleMsg => {
                if !self.check_friend_relation(&msg.send_id, &msg.receiver_id).await? {
                    return Err(Status::permission_denied("无法发送消息：用户不是好友或已被拉黑"));
                }
            }

            // 群聊消息需要校验群是否存在和群组成员身份
            MsgType::GroupMsg => {
                // 检查群组是否存在
                if !self.check_group_exists(&msg.group_id).await? {
                    return Err(Status::permission_denied("无法发送消息：群组不存在"));
                }

                // 对于群聊消息，receiver_id 应该是群组ID
                let group_id = if !msg.group_id.is_empty() {
                    &msg.group_id
                } else {
                    &msg.receiver_id
                };

                if !self.check_group_membership(&msg.send_id, group_id).await? {
                    return Err(Status::permission_denied("无法发送消息：用户不是群组成员"));
                }
            }

            // 好友相关消息类型（好友申请、响应等）需要检查拉黑状态
            MsgType::FriendApplyReq | MsgType::FriendApplyResp => {
                // 检查是否被拉黑（不检查好友关系，因为这些消息可能在非好友间发送）
                if self.check_blocked_status(&msg.receiver_id, &msg.send_id).await? {
                    return Err(Status::permission_denied("无法发送消息：已被对方拉黑"));
                }
            }

            // 通话相关消息需要校验好友关系
            MsgType::SingleCallInvite | MsgType::RejectSingleCall | MsgType::AgreeSingleCall |
            MsgType::SingleCallInviteNotAnswer | MsgType::SingleCallInviteCancel |
            MsgType::SingleCallOffer | MsgType::Hangup | MsgType::ConnectSingleCall |
            MsgType::Candidate => {
                if !self.check_friend_relation(&msg.send_id, &msg.receiver_id).await? {
                    return Err(Status::permission_denied("无法发起通话：用户不是好友或已被拉黑"));
                }
            }

            // 系统消息和其他特殊消息类型暂不校验
            _ => {
                debug!("消息类型 {:?} 无需权限校验", msg_type);
            }
        }

        Ok(())
    }

    /// 检查群组是否存在
    ///
    /// # 返回值
    /// * `Ok(true)` - 群组存在
    /// * `Ok(false)` - 群组不存在
    /// * `Err(Status)` - 检查失败
    async fn check_group_exists(&self, group_id: &str) -> Result<bool, Status> {
        // 使用RPC调用检查群组是否存在
        let mut group_client = self.group_client.clone();
        match group_client.get_group(GetGroupRequest {
            group_id: group_id.to_string(),
        }).await {
            Ok(_) => {
                debug!("群组 {} 存在（RPC确认）", group_id);
                Ok(true)
            }
            Err(e) => {
                if e.code() == tonic::Code::NotFound {
                    debug!("群组 {} 不存在（RPC确认）", group_id);
                    Ok(false)
                } else {
                    error!("检查群组存在性失败: {}", e);
                    Err(Status::internal("检查群组存在性失败"))
                }
            }
        }
    }

    /// 检查消息是否允许转发
    pub(super) async fn is_message_forwardable(&self, msg: &Msg) -> Result<bool, Status> {
        let msg_type = MsgType::try_from(msg.msg_type)
            .map_err(|_| Status::invalid_argument("无效的消息类型"))?;

        // 检查消息类型是否允许转发
        match msg_type {
            // 允许转发的消息类型
            MsgType::SingleMsg | MsgType::GroupMsg => {
                // 检查消息是否已被撤回
                if msg.is_revoked {
                    return Ok(false);
                }
                Ok(true)
            }

            // 不允许转发的消息类型
            MsgType::FriendApplyReq | MsgType::FriendApplyResp |
            MsgType::GroupInvitation | MsgType::GroupDismiss |
            MsgType::GroupMemberExit | MsgType::GroupRemoveMember |
            MsgType::SingleCallInvite | MsgType::RejectSingleCall |
            MsgType::AgreeSingleCall | MsgType::SingleCallInviteNotAnswer |
            MsgType::SingleCallInviteCancel | MsgType::SingleCallOffer |
            MsgType::Hangup | MsgType::ConnectSingleCall | MsgType::Candidate |
            MsgType::Read => {
                Ok(false)
            }

            // 其他消息类型默认不允许转发
            _ => {
                debug!("消息类型 {:?} 默认不允许转发", msg_type);
                Ok(false)
            }
        }
    }

    /// 检查转发到用户的权限
    pub(super) async fn check_forward_to_user_permission(&self, sender_id: &str, target_user_id: &str) -> Result<(), Status> {
        // 检查好友关系
        if !self.check_friend_relation(sender_id, target_user_id).await? {
            return Err(Status::permission_denied("无法转发：用户不是好友或已被拉黑"));
        }
        Ok(())
    }

    /// 检查转发到群组的权限
    pub(super) async fn check_forward_to_group_permission(&self, sender_id: &str, target_group_id: &str) -> Result<(), Status> {
        // 检查群组是否存在
        if !self.check_group_exists(target_group_id).await? {
            return Err(Status::permission_denied("无法转发：群组不存在"));
        }

        // 检查用户是否是群组成员
        if !self.check_group_membership(sender_id, target_group_id).await? {
            return Err(Status::permission_denied("无法转发：用户不是群组成员"));
        }

        // TODO: 可以添加更多群组权限检查，如：
        // - 检查用户是否被禁言
        // - 检查群组是否允许转发消息
        // - 检查群组设置等

        Ok(())
    }
}
