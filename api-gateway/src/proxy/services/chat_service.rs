use super::common::{error_response, extract_i64_param, extract_string_array_param, extract_string_param, get_i64_param, get_optional_string, get_platform_from_jwt, get_user_id_from_jwt, success_response};
use axum::{body::Body, http::{Method, Response, StatusCode}};
use chrono::Utc;
use common::proto::message::chat_service_client::ChatServiceClient;
use common::proto::message::{ContentType, DeleteMessagesRequest, ForwardMessageRequest, GetConversationsRequest, GetDbMessagesRequest, MarkMessagesAsReadRequest, MarkConversationAsReadRequest, Msg, MsgType, RevokeMessageRequest, SendMsgRequest, GetConversationsResponse};
use common::service_discovery::LbWithServiceDiscovery;
use serde_json::Value;
use tracing::{debug, error};
use common::auth::Claims;

/// 聊天服务处理器
#[derive(Clone)]
pub struct ChatServiceHandler {
    client: ChatServiceClient<LbWithServiceDiscovery>,
}

impl ChatServiceHandler {
    /// 创建新的聊天服务处理器
    pub fn new(client: ChatServiceClient<LbWithServiceDiscovery>) -> Self {
        Self { client }
    }

    /// 处理聊天服务请求
    pub async fn handle_request(
        &mut self,
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<Claims>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理聊天服务请求: {} {}", method, path);

        // 从JWT中获取用户ID
        let current_user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;
        
        // 从JWT中获取当前登录平台
        let current_platform = get_platform_from_jwt(jwt_user_info.as_ref())?;

        // 从路径提取方法名 - 格式: /api/chat/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 发送消息（统一接口，支持单聊/群聊/回复）
            (&Method::POST, "send") => self.send_message(&current_user_id, body, current_platform).await,

            // 标记消息已读
            (&Method::POST, "read") | (&Method::POST, "markAsRead") => {
                self.mark_messages_as_read(&current_user_id, body).await
            }

            // 标记会话已读
            (&Method::POST, "markConversationAsRead") => {
                self.mark_conversation_as_read(&current_user_id, body).await
            }

            // 获取消息历史
            (&Method::POST, "history") => self.get_message_history(&current_user_id, body).await,

            // 获取会话列表
            (&Method::POST, "conversations") => self.get_conversations(&current_user_id, body).await,

            // 撤回消息
            (&Method::POST, "revoke") => self.revoke_message(&current_user_id, body).await,

            // 删除消息
            (&Method::POST, "delete") | (&Method::DELETE, "messages") => {
                self.delete_messages(&current_user_id, body).await
            }

            // 转发消息
            (&Method::POST, "forward") => self.forward_message(&current_user_id, body).await,

            _ => {
                error!("不支持的聊天服务方法: {} {}", method, method_name);
                Ok(error_response(
                    &format!("不支持的方法: {}", method_name),
                    StatusCode::NOT_FOUND,
                ))
            }
        }
    }

    /// 统一发送消息接口
    /// 支持单聊、群聊、回复消息，通过msgType参数区分
    async fn send_message(
        &mut self,
        sender_id: &str,
        body: Value,
        platform: i32,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("发送消息请求: {}", body);

        // 提取消息类型
        let msg_type = get_i64_param(&body, "msgType", 28) as i32;

        // 提取必需参数
        let receiver_id = extract_string_param(&body, "receiverId", Some("receiver_id"))?;
        let content = extract_string_param(&body, "content", Some("content"))?;
        let local_id = extract_string_param(&body, "localId", Some("localId"))?;
        let content_type_str = extract_string_param(&body, "contentType", Some("content_type"))?;
        
        // 提取可选参数
        let related_msg_id = get_optional_string(&body, "relatedMsgId", Some("related_msg_id"));
        let avatar = get_optional_string(&body, "avatar", Some("avatar"));
        let nickname = get_optional_string(&body, "nickname", Some("nickname"));

        if content.trim().is_empty() {
            return Ok(error_response("消息内容不能为空", StatusCode::BAD_REQUEST));
        }

        if content.len() > 2048 {
            return Ok(error_response(
                "消息内容不能超过2048字符",
                StatusCode::BAD_REQUEST,
            ));
        }

        // 解析内容类型
        let content_type = ContentType::from_str_name(&content_type_str)
            .map(|ct| ct as i32)
            .unwrap_or(ContentType::Text as i32);

        // 构建消息对象
        let msg = Msg {
            send_id: sender_id.to_string(),
            receiver_id: receiver_id.to_string(),
            local_id,
            server_id: String::new(), // 由msg-server生成
            create_time: Utc::now().timestamp_millis(),
            send_time: 0, // 由msg-server设置
            seq: 0,       // 由消费者服务设置
            msg_type,
            content_type,
            content: content.into_bytes(),
            is_read: false,
            group_id: if msg_type == 1 { receiver_id.clone() } else { String::new() },
            platform,
            avatar: avatar.map_or_else(|| String::new(), |s| s),
            nickname: nickname.map_or_else(|| String::new(), |s| s),
            related_msg_id,
            send_seq: 0,
            is_revoked: false,
            revoke_time: 0,
            revoked_by: String::new(),
        };


        // 调用gRPC服务发送消息
        let request = SendMsgRequest { message: Some(msg) };

        match self.client.send_msg(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("消息发送失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }


    /// 标记消息已读
    async fn mark_messages_as_read(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("标记消息已读请求: {}", body);

        // 提取消息序列号列表
        let msg_seqs = body
            .get("msgSeqs")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("msgSeqs参数是必需的"))?
            .iter()
            .filter_map(|v| v.as_i64())
            .collect::<Vec<i64>>();

        if msg_seqs.is_empty() {
            return Ok(error_response(
                "消息序列号列表不能为空",
                StatusCode::BAD_REQUEST,
            ));
        }

        // 调用msg-server的gRPC接口标记消息已读
        let request = MarkMessagesAsReadRequest {
            user_id: user_id.to_string(),
            msg_seqs,
        };

        match self.client.mark_messages_as_read(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("标记消息已读失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 标记会话已读
    async fn mark_conversation_as_read(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("标记会话已读请求: {}", body);

        // 提取参数
        let conversation_id = extract_string_param(&body, "conversationId", Some("conversation_id"))?;
        let up_to_time = body
            .get("upToTime")
            .or_else(|| body.get("up_to_time"))
            .and_then(|v| v.as_i64());

        // 验证参数
        if conversation_id.is_empty() {
            return Ok(error_response("会话ID不能为空", StatusCode::BAD_REQUEST));
        }

        // 调用msg-server的gRPC接口标记会话已读
        let request = MarkConversationAsReadRequest {
            user_id: user_id.to_string(),
            conversation_id,
            up_to_time,
        };

        match self.client.mark_conversation_as_read(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("标记会话已读失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 获取消息历史
    /// 根据会话ID和序列号范围获取消息历史
    async fn get_message_history(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("获取消息历史请求: {}", body);

        // 提取必需参数
        let conversation_id = extract_string_param(&body, "conversationId", Some("conversation_id"))?;

        // 提取序列号范围参数
        let send_seq_start = extract_i64_param(&body, "sendSeqStart", Some("send_seq_start"))?;
        let send_seq_end = extract_i64_param(&body, "sendSeqEnd", Some("send_seq_end"))?;
        let seq_start = extract_i64_param(&body, "seqStart", Some("seq_start"))?;
        let seq_end = extract_i64_param(&body, "seqEnd", Some("seq_end"))?;

        if send_seq_start < 0 || send_seq_end < 0 || seq_start < 0 || seq_end < 0 {
            return Ok(error_response("序列号不能为负数", StatusCode::BAD_REQUEST));
        }

        if send_seq_end > 0 && send_seq_end < send_seq_start {
            return Ok(error_response("发送序列号结束值不能小于起始值", StatusCode::BAD_REQUEST));
        }

        if seq_end > 0 && seq_end < seq_start {
            return Ok(error_response("接收序列号结束值不能小于起始值", StatusCode::BAD_REQUEST));
        }

        debug!("序列号范围: send_seq: {}-{}, seq: {}-{}", 
               send_seq_start, send_seq_end, seq_start, seq_end);

        // 构建gRPC请求
        let request = GetDbMessagesRequest {
            user_id: user_id.to_string(),
            conversation_id,
            send_seq_start,
            send_seq_end,
            seq_start,
            seq_end,
        };

        match self.client.get_message_history(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("获取消息历史失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 获取会话列表
    async fn get_conversations(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("获取会话列表请求，用户ID: {}", user_id);

        // 获取离线同步参数
        let sync_mode = body.get("syncMode").and_then(|v| v.as_bool()).unwrap_or(false);
        let since_seq = if sync_mode {
            body.get("sinceSeq").and_then(|v| v.as_i64())
        } else {
            None
        };
        let since_send_seq = if sync_mode {
            body.get("sinceSendSeq").and_then(|v| v.as_i64()) 
        } else {
            None
        };

        debug!("同步参数: sync_mode={}, since_seq={:?}, since_send_seq={:?}", 
               sync_mode, since_seq, since_send_seq);

        // 构建gRPC请求
        let request = GetConversationsRequest {
            user_id: user_id.to_string(),
            since_seq,
            since_send_seq,
            sync_mode: Some(sync_mode),
        };

        match self.client.get_conversations(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("获取会话列表失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 撤回消息
    async fn revoke_message(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("撤回消息请求: {}", body);

        // 提取消息ID
        let message_id = extract_string_param(&body, "messageId", Some("message_id"))?;

        // 调用msg-server的gRPC接口撤回消息
        let request = RevokeMessageRequest {
            user_id: user_id.to_string(),
            message_id,
        };

        match self.client.revoke_message(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("撤回消息失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 删除消息
    async fn delete_messages(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("删除消息请求: {}", body);

        // 提取消息ID列表或序列号列表
        let message_ids = extract_string_array_param(&body, "messageIds", Some("message_ids"))?;

        let message_seqs = body
            .get("messageSeqs")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>())
            .unwrap_or_default();

        // 验证参数
        if message_ids.is_empty() && message_seqs.is_empty() {
            return Ok(error_response(
                "必须提供消息ID或消息序列号",
                StatusCode::BAD_REQUEST,
            ));
        }

        // 调用msg-server的gRPC接口删除消息
        let request = DeleteMessagesRequest {
            user_id: user_id.to_string(),
            message_ids,
            message_seqs,
        };

        match self.client.delete_messages(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("删除消息失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 转发消息
    async fn forward_message(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("转发消息请求: {}", body);

        // 解析请求参数
        let message_ids: Vec<String> = extract_string_array_param(&body, "messageIds", Some("message_ids"))?;
        let target_user_ids: Vec<String> = extract_string_array_param(&body, "targetUserIds", Some("target_user_ids"))?;
        let target_group_ids: Vec<String> = extract_string_array_param(&body, "targetGroupIds", Some("target_group_ids"))?;
        let forward_comment = get_optional_string(&body, "comment", Some("comment"));

        // 验证参数
        if message_ids.is_empty() {
            return Ok(error_response("原始消息ID不能为空", StatusCode::BAD_REQUEST));
        }

        if target_user_ids.is_empty() && target_group_ids.is_empty() {
            return Ok(error_response("必须指定转发目标", StatusCode::BAD_REQUEST));
        }

        // 构建gRPC请求
        let request = ForwardMessageRequest {
            user_id: user_id.to_string(),
            message_ids,
            target_user_ids,
            target_group_ids,
            forward_comment,
        };

        // 调用msg-server
        match self.client.forward_message(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(e) => {
                error!("转发消息失败: {}", e);
                Ok(error_response("转发消息失败", StatusCode::INTERNAL_SERVER_ERROR))
            }
        }
    }
}
