use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::grpc_client::ChatServiceGrpcClient;
use common::message::{Msg, SendMsgRequest, MsgType, ContentType, PlatformType};
use serde_json::{json, Value};
use tracing::{error, debug};
use chrono::Utc;

use super::common::{
    success_response, error_response, extract_string_param, get_optional_string, 
    get_i64_param, get_user_id_from_jwt,
};
use crate::auth::jwt::UserInfo;

/// 聊天服务处理器
#[derive(Clone)]
pub struct ChatServiceHandler {
    client: ChatServiceGrpcClient,
}

impl ChatServiceHandler {
    /// 创建新的聊天服务处理器
    pub fn new(client: ChatServiceGrpcClient) -> Self {
        Self { client }
    }

    /// 处理聊天服务请求
    pub async fn handle_request(
        &mut self,
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<UserInfo>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理聊天服务请求: {} {}", method, path);

        // 从JWT中获取用户ID
        let current_user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;

        // 从路径提取方法名 - 格式: /api/chat/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 发送单聊消息
            (&Method::POST, "send") | (&Method::POST, "sendMessage") => {
                self.send_message(&current_user_id, body).await
            }

            // 发送群聊消息
            (&Method::POST, "sendGroup") | (&Method::POST, "sendGroupMessage") => {
                self.send_group_message(&current_user_id, body).await
            }

            // 标记消息已读
            (&Method::POST, "read") | (&Method::POST, "markAsRead") => {
                self.mark_messages_as_read(&current_user_id, body).await
            }

            // 获取消息历史
            (&Method::GET, "history") => {
                self.get_message_history(&current_user_id, body).await
            }

            // 获取会话列表
            (&Method::GET, "conversations") => {
                self.get_conversations(&current_user_id, body).await
            }

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
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("发送单聊消息请求: {}", body);

        // 提取必需参数
        let receiver_id = extract_string_param(&body, "receiverId", Some("receiver_id"))?;
        let content = extract_string_param(&body, "content", Some("content"))?;
        
        // 提取可选参数
        let content_type_str = get_optional_string(&body, "contentType", Some("content_type"))
            .unwrap_or_else(|| "Text".to_string());
        let platform_str = get_optional_string(&body, "platform", Some("platform"))
            .unwrap_or_else(|| "Desktop".to_string());
        let local_id = get_optional_string(&body, "localId", Some("local_id"))
            .unwrap_or_else(|| format!("local_{}", Utc::now().timestamp_millis()));
        let related_msg_id = get_optional_string(&body, "relatedMsgId", Some("related_msg_id"));

        // 验证参数
        if receiver_id == sender_id {
            return Ok(error_response("不能给自己发送消息", StatusCode::BAD_REQUEST));
        }

        if content.trim().is_empty() {
            return Ok(error_response("消息内容不能为空", StatusCode::BAD_REQUEST));
        }

        if content.len() > 2048 {
            return Ok(error_response("消息内容不能超过2048字符", StatusCode::BAD_REQUEST));
        }

        // 解析内容类型
        let content_type = match content_type_str.as_str() {
            "Text" => ContentType::Text as i32,
            "Image" => ContentType::Image as i32,
            "Audio" => ContentType::Audio as i32,
            "Video" => ContentType::Video as i32,
            "File" => ContentType::File as i32,
            _ => ContentType::Text as i32,
        };

        // 解析平台类型
        let platform = match platform_str.as_str() {
            "Desktop" => PlatformType::Desktop as i32,
            "Mobile" => PlatformType::Mobile as i32,
            _ => PlatformType::Desktop as i32,
        };

        // 构建消息对象
        let msg = Msg {
            send_id: sender_id.to_string(),
            receiver_id: receiver_id.clone(),
            local_id,
            server_id: String::new(), // 由msg-server生成
            create_time: Utc::now().timestamp_millis(),
            send_time: 0, // 由msg-server设置
            seq: 0, // 由消费者服务设置
            msg_type: MsgType::SingleMsg as i32,
            content_type,
            content: content.into_bytes(),
            is_read: false,
            group_id: String::new(),
            platform,
            avatar: String::new(), // 可以从用户信息中获取
            nickname: String::new(), // 可以从用户信息中获取
            related_msg_id,
            send_seq: 0, // 由msg-gateway设置
        };

        // 调用gRPC服务发送消息
        let request = SendMsgRequest {
            message: Some(msg),
        };

        match self.client.send_msg(request).await {
            Ok(response) => {
                let msg_response = response;
                
                if msg_response.err.is_empty() {
                    // 发送成功
                    let response_data = json!({
                        "localId": msg_response.local_id,
                        "serverId": msg_response.server_id,
                        "sendTime": msg_response.send_time,
                        "status": "sent"
                    });
                    Ok(success_response(response_data, StatusCode::OK))
                } else {
                    // 发送失败
                    error!("消息发送失败: {}", msg_response.err);
                    Ok(error_response(&msg_response.err, StatusCode::INTERNAL_SERVER_ERROR))
                }
            }
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("消息发送失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 发送群聊消息
    async fn send_group_message(
        &mut self,
        sender_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("发送群聊消息请求: {}", body);

        // 提取必需参数
        let group_id = extract_string_param(&body, "groupId", Some("group_id"))?;
        let content = extract_string_param(&body, "content", Some("content"))?;
        
        // 提取可选参数
        let content_type_str = get_optional_string(&body, "contentType", Some("content_type"))
            .unwrap_or_else(|| "Text".to_string());
        let platform_str = get_optional_string(&body, "platform", Some("platform"))
            .unwrap_or_else(|| "Desktop".to_string());
        let local_id = get_optional_string(&body, "localId", Some("local_id"))
            .unwrap_or_else(|| format!("local_{}", Utc::now().timestamp_millis()));
        let related_msg_id = get_optional_string(&body, "relatedMsgId", Some("related_msg_id"));

        // 验证参数
        if content.trim().is_empty() {
            return Ok(error_response("消息内容不能为空", StatusCode::BAD_REQUEST));
        }

        if content.len() > 2048 {
            return Ok(error_response("消息内容不能超过2048字符", StatusCode::BAD_REQUEST));
        }

        // 解析内容类型
        let content_type = match content_type_str.as_str() {
            "Text" => ContentType::Text as i32,
            "Image" => ContentType::Image as i32,
            "Audio" => ContentType::Audio as i32,
            "Video" => ContentType::Video as i32,
            "File" => ContentType::File as i32,
            _ => ContentType::Text as i32,
        };

        // 解析平台类型
        let platform = match platform_str.as_str() {
            "Desktop" => PlatformType::Desktop as i32,
            "Mobile" => PlatformType::Mobile as i32,
            _ => PlatformType::Desktop as i32,
        };

        // 构建群聊消息对象
        let msg = Msg {
            send_id: sender_id.to_string(),
            receiver_id: group_id.clone(),
            local_id,
            server_id: String::new(), // 由msg-server生成
            create_time: Utc::now().timestamp_millis(),
            send_time: 0, // 由msg-server设置
            seq: 0, // 由消费者服务设置
            msg_type: MsgType::GroupMsg as i32,
            content_type,
            content: content.into_bytes(),
            is_read: false,
            group_id: group_id.clone(),
            platform,
            avatar: String::new(), // 可以从用户信息中获取
            nickname: String::new(), // 可以从用户信息中获取
            related_msg_id,
            send_seq: 0, // 由msg-gateway设置
        };

        // 调用gRPC服务发送群聊消息
        let request = SendMsgRequest {
            message: Some(msg),
        };

        match self.client.send_msg(request).await {
            Ok(response) => {
                let msg_response = response;
                
                if msg_response.err.is_empty() {
                    // 发送成功
                    let response_data = json!({
                        "localId": msg_response.local_id,
                        "serverId": msg_response.server_id,
                        "sendTime": msg_response.send_time,
                        "status": "sent"
                    });
                    Ok(success_response(response_data, StatusCode::OK))
                } else {
                    // 发送失败
                    error!("群聊消息发送失败: {}", msg_response.err);
                    Ok(error_response(&msg_response.err, StatusCode::INTERNAL_SERVER_ERROR))
                }
            }
            Err(err) => {
                error!("调用聊天服务失败: {}", err);
                Ok(error_response(
                    &format!("群聊消息发送失败: {}", err),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
    }

    /// 标记消息已读
    async fn mark_messages_as_read(
        &mut self,
        _user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("标记消息已读请求: {}", body);

        // 提取消息序列号列表
        let msg_seqs = body.get("msgSeqs")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("msgSeqs参数是必需的"))?
            .iter()
            .filter_map(|v| v.as_i64())
            .collect::<Vec<i64>>();

        if msg_seqs.is_empty() {
            return Ok(error_response("消息序列号列表不能为空", StatusCode::BAD_REQUEST));
        }

        // TODO: 实现消息已读功能
        // 这里需要调用消息存储服务来标记消息已读
        // 暂时返回成功响应
        
        let response_data = json!({
            "success": true,
            "readCount": msg_seqs.len()
        });
        
        Ok(success_response(response_data, StatusCode::OK))
    }

    /// 获取消息历史
    async fn get_message_history(
        &mut self,
        _user_id: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("获取消息历史请求: {}", body);

        // 提取参数
        let conversation_id = get_optional_string(&body, "conversationId", Some("conversation_id"))
            .ok_or_else(|| anyhow::anyhow!("conversationId参数是必需的"))?;
        let page = get_i64_param(&body, "page", 1);
        let page_size = get_i64_param(&body, "pageSize", 20);
        let _before_time = get_i64_param(&body, "beforeTime", 0);

        // 验证参数
        if page < 1 {
            return Ok(error_response("页码必须大于0", StatusCode::BAD_REQUEST));
        }

        if page_size < 1 || page_size > 100 {
            return Ok(error_response("每页数量必须在1-100之间", StatusCode::BAD_REQUEST));
        }

        // TODO: 实现获取消息历史功能
        // 这里需要调用消息存储服务来获取历史消息
        // 暂时返回空列表
        
        let response_data = json!({
            "messages": [],
            "hasMore": false,
            "nextTime": null,
            "conversationId": conversation_id
        });
        
        Ok(success_response(response_data, StatusCode::OK))
    }

    /// 获取会话列表
    async fn get_conversations(
        &mut self,
        _user_id: &str,
        _body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("获取会话列表请求");

        // TODO: 实现获取会话列表功能
        // 这里需要调用消息存储服务来获取用户的会话列表
        // 暂时返回空列表
        
        let response_data = json!({
            "conversations": []
        });
        
        Ok(success_response(response_data, StatusCode::OK))
    }
} 