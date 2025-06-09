use super::common::{error_response, extract_string_param, get_i64_param, get_optional_string, get_platform_from_jwt, get_user_id_from_jwt, success_response};
use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use chrono::Utc;
use common::proto::message::chat_service_client::ChatServiceClient;
use common::proto::message::{
    ContentType, DeleteMessagesRequest, ForwardMessageRequest, GetConversationsRequest,
    GetMessageHistoryRequest, MarkMessagesAsReadRequest, MarkConversationAsReadRequest, Msg, MsgType, PlatformType,
    ReplyMessageRequest, RevokeMessageRequest, SendMsgRequest,
};
use common::service_discovery::LbWithServiceDiscovery;
use serde_json::{json, Value};
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
            // 发送单聊消息
            (&Method::POST, "send") => self.send_message(&current_user_id, body, current_platform).await,

            // 发送群聊消息
            (&Method::POST, "sendGroup") => self.send_group_message(&current_user_id, body, current_platform).await,

            // 标记消息已读
            (&Method::POST, "read") | (&Method::POST, "markAsRead") => {
                self.mark_messages_as_read(&current_user_id, body).await
            }

            // 标记会话已读
            (&Method::POST, "markConversationAsRead") => {
                self.mark_conversation_as_read(&current_user_id, body).await
            }

            // 获取消息历史
            (&Method::GET, "history") => self.get_message_history(&current_user_id, body).await,

            // 获取会话列表
            (&Method::GET, "conversations") => self.get_conversations(&current_user_id, body).await,

            // 拉取离线消息
            (&Method::GET, "pull_offline_messages") => {
                self.pull_offline_messages(&current_user_id, body).await
            }

            // 撤回消息
            (&Method::POST, "revoke") => self.revoke_message(&current_user_id, body).await,

            // 删除消息
            (&Method::POST, "delete") | (&Method::DELETE, "messages") => {
                self.delete_messages(&current_user_id, body).await
            }

            // 转发消息
            (&Method::POST, "forward") => self.forward_message(&current_user_id, body).await,

            // 回复消息
            (&Method::POST, "reply") => self.reply_message(&current_user_id, body).await,

            _ => {
                error!("不支持的聊天服务方法: {} {}", method, method_name);
                Ok(error_response(
                    &format!("不支持的方法: {}", method_name),
                    StatusCode::NOT_FOUND,
                ))
            }
        }
    }

    /// 发送单聊消息
    async fn send_message(
        &mut self,
        sender_id: &str,
        body: Value,
        platform: i32,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("发送单聊消息请求: {}", body);

        // 提取必需参数
        let receiver_id = extract_string_param(&body, "receiverId", Some("receiver_id"))?;
        let content = extract_string_param(&body, "content", Some("content"))?;
        let local_id = extract_string_param(&body, "localId", Some("localId"))?;
        let content_type_str = extract_string_param(&body, "contentType", Some("content_type"))?;
        
        // 提取可选参数
        let related_msg_id = get_optional_string(&body, "relatedMsgId", Some("related_msg_id"));

        // 验证参数
        if receiver_id == sender_id {
            return Ok(error_response(
                "不能给自己发送消息",
                StatusCode::BAD_REQUEST,
            ));
        }

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
            receiver_id: receiver_id.clone(),
            local_id,
            server_id: String::new(), // 由msg-server生成
            create_time: Utc::now().timestamp_millis(),
            send_time: 0, // 由msg-server设置
            seq: 0,       // 由消费者服务设置
            msg_type: MsgType::SingleMsg as i32,
            content_type,
            content: content.into_bytes(),
            is_read: false,
            group_id: String::new(),
            platform,
            avatar: String::new(),   // 可以从用户信息中获取
            nickname: String::new(), // 可以从用户信息中获取
            related_msg_id,
            send_seq: 0, // 由msg-gateway设置
            is_revoked: false,
            revoke_time: 0,
            revoked_by: String::new(),
            forward_comment: None,
            is_forwarded: false,
            is_reply: false,
        };

        // 调用gRPC服务发送消息
        let request = SendMsgRequest { message: Some(msg) };

        match self.client.send_msg(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("消息发送失败: {}", err),
                    StatusCode::SERVICE_UNAVAILABLE,
                ))
            }
        }
    }

    /// 发送群聊消息
    async fn send_group_message(
        &mut self,
        sender_id: &str,
        body: Value,
        platform: i32,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("发送群聊消息请求: {}", body);

        // 提取必需参数
        let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
        let content = extract_string_param(&body, "content", Some("content"))?;
        let local_id = extract_string_param(&body, "localId", Some("localId"))?;
        let content_type_str = extract_string_param(&body, "contentType", Some("content_type"))?;

        // 提取可选参数
        let related_msg_id = get_optional_string(&body, "relatedMsgId", Some("related_msg_id"));

        // 验证参数
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

        // 构建群聊消息对象
        let msg = Msg {
            send_id: sender_id.to_string(),
            receiver_id: group_id.clone(),
            local_id,
            server_id: String::new(), // 由msg-server生成
            create_time: Utc::now().timestamp_millis(),
            send_time: 0, // 由msg-server设置
            seq: 0,       // 由消费者服务设置
            msg_type: MsgType::GroupMsg as i32,
            content_type,
            content: content.into_bytes(),
            is_read: false,
            group_id: group_id.clone(),
            platform,
            avatar: String::new(),   // 可以从用户信息中获取
            nickname: String::new(), // 可以从用户信息中获取
            related_msg_id,
            send_seq: 0, // 由msg-gateway设置
            is_revoked: false,
            revoke_time: 0,
            revoked_by: String::new(),
            forward_comment: None,
            is_forwarded: false,
            is_reply: false,
        };

        // 调用gRPC服务发送群聊消息
        let request = SendMsgRequest { message: Some(msg) };

        match self.client.send_msg(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("群聊消息发送失败: {}", err),
                    StatusCode::SERVICE_UNAVAILABLE,
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
                    StatusCode::SERVICE_UNAVAILABLE,
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
                    StatusCode::SERVICE_UNAVAILABLE,
                ))
            }
        }
    }

    /// 获取消息历史
    async fn get_message_history(
        &mut self,
        user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("获取消息历史请求: {}", body);

        // 提取参数
        let conversation_id = get_optional_string(&body, "conversationId", Some("conversation_id"))
            .ok_or_else(|| anyhow::anyhow!("conversationId参数是必需的"))?;
        let page = get_i64_param(&body, "page", 1) as i32;
        let page_size = get_i64_param(&body, "pageSize", 20) as i32;
        let before_seq = get_i64_param(&body, "beforeSeq", 0);

        // 验证参数
        if page < 1 {
            return Ok(error_response("页码必须大于0", StatusCode::BAD_REQUEST));
        }

        if page_size < 1 || page_size > 100 {
            return Ok(error_response(
                "每页数量必须在1-100之间",
                StatusCode::BAD_REQUEST,
            ));
        }

        // 调用msg-server的gRPC接口获取消息历史
        let request = GetMessageHistoryRequest {
            user_id: user_id.to_string(),
            conversation_id: conversation_id.clone(),
            page,
            page_size,
            before_seq,
        };

        match self.client.get_message_history(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("获取消息历史失败: {}", err),
                    StatusCode::SERVICE_UNAVAILABLE,
                ))
            }
        }
    }

    /// 获取会话列表
    async fn get_conversations(
        &mut self,
        user_id: &str,
        _body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("获取会话列表请求，用户ID: {}", user_id);

        // 调用msg-server的gRPC接口获取会话列表
        let request = GetConversationsRequest {
            user_id: user_id.to_string(),
        };

        match self.client.get_conversations(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("获取会话列表失败: {}", err),
                    StatusCode::SERVICE_UNAVAILABLE,
                ))
            }
        }
    }

    /// 拉取离线消息
    async fn pull_offline_messages(
        &mut self,
        user_id: &str,
        _body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("拉取离线消息请求，用户ID: {}", user_id);
        Ok(error_response("暂不支持离线消息拉取", StatusCode::NOT_IMPLEMENTED))
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
                    StatusCode::SERVICE_UNAVAILABLE,
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
        let message_ids = body
            .get("messageIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

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
                    StatusCode::SERVICE_UNAVAILABLE,
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
        let original_message_id = body["messageId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("缺少原始消息ID"))?;

        let target_user_ids: Vec<String> = body["targetUserIds"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let target_group_ids: Vec<String> = body["targetGroupIds"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let forward_comment = body["comment"].as_str().map(|s| s.to_string());

        // 验证参数
        if original_message_id.is_empty() {
            return Ok(error_response("原始消息ID不能为空", StatusCode::BAD_REQUEST));
        }

        if target_user_ids.is_empty() && target_group_ids.is_empty() {
            return Ok(error_response("必须指定转发目标", StatusCode::BAD_REQUEST));
        }

        // 构建gRPC请求
        let request = ForwardMessageRequest {
            user_id: user_id.to_string(),
            original_message_id: original_message_id.to_string(),
            target_user_ids,
            target_group_ids,
            forward_comment,
        };

        // 调用msg-server
        match self.client.forward_message(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(e) => {
                error!("转发消息失败: {}", e);
                Ok(error_response("转发消息失败", StatusCode::SERVICE_UNAVAILABLE))
            }
        }
    }

    /// 回复消息
    async fn reply_message(&mut self, user_id: &str, body: Value) -> Result<Response<Body>, anyhow::Error> {
        debug!("回复消息请求: {}", body);

        // 解析请求参数
        let original_message_id = body["originalMessageId"]
            .as_str()
            .or_else(|| body["messageId"].as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少原始消息ID"))?;

        let reply_content = body["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("缺少回复内容"))?;

        let content_type_str = body["contentType"].as_str().unwrap_or("Text");

        let conversation_id = body["conversationId"].as_str().map(|s| s.to_string());

        // 验证参数
        if original_message_id.is_empty() {
            return Ok(error_response("原始消息ID不能为空", StatusCode::BAD_REQUEST));
        }

        if reply_content.trim().is_empty() {
            return Ok(error_response("回复内容不能为空", StatusCode::BAD_REQUEST));
        }

        if reply_content.len() > 2048 {
            return Ok(error_response("回复内容不能超过2048字符", StatusCode::BAD_REQUEST));
        }

        // 解析内容类型
        let content_type = ContentType::from_str_name(&content_type_str)
            .map(|ct| ct as i32)
            .unwrap_or(ContentType::Text as i32);

        // 构建gRPC请求
        let request = ReplyMessageRequest {
            user_id: user_id.to_string(),
            original_message_id: original_message_id.to_string(),
            reply_content: reply_content.to_string(),
            content_type,
            conversation_id,
        };

        // 调用msg-server
        match self.client.reply_message(request).await {
            Ok(response) => Ok(success_response(response.into_inner(), StatusCode::OK)),
            Err(e) => {
                error!("回复消息失败: {}", e);
                Ok(error_response("回复消息失败", StatusCode::SERVICE_UNAVAILABLE))
            }
        }
    }
}
