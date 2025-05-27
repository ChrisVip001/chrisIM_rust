use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use common::error::Error;
use common::message::{
    DelMsgRequest, GetDbMsgRequest, GetDbMessagesRequest, MsgReadReq, MsgReadResp, 
    SaveMaxSeqRequest, SaveGroupMsgRequest, SaveMessageRequest, Msg
};
use msg_storage::{DbRepo, message::MsgRecBoxRepo};

// 注意: 这些类型需要在proto文件生成后导入
// use common::msg_storage::{
//     MessageStorageServiceServer, MessageStorageService,
//     SaveMessageResponse, GetMessagesResponse, DeleteMessageResponse,
//     GetMessageRequest, GetMessageResponse, SaveMaxSeqResponse,
//     HealthCheckRequest, HealthCheckResponse
// };

/// 消息存储RPC服务实现
/// 负责处理消息的存储、查询、删除等操作
#[derive(Debug)]
pub struct MessageStorageServiceImpl {
    /// 数据库仓库，包含PostgreSQL和序列号管理
    db_repo: Arc<DbRepo>,
    /// MongoDB消息盒子仓库，用于离线消息存储
    msg_box_repo: Arc<dyn MsgRecBoxRepo>,
}

impl MessageStorageServiceImpl {
    /// 创建新的消息存储服务实例
    /// 
    /// # 参数
    /// * `db_repo` - 数据库仓库
    /// * `msg_box_repo` - 消息盒子仓库
    /// 
    /// # 返回值
    /// 返回消息存储服务实例
    pub fn new(
        db_repo: Arc<DbRepo>, 
        msg_box_repo: Arc<dyn MsgRecBoxRepo>
    ) -> Self {
        Self {
            db_repo,
            msg_box_repo,
        }
    }

    /// 验证消息请求的合法性
    /// 
    /// # 参数
    /// * `message` - 要验证的消息
    /// 
    /// # 返回值
    /// 验证成功返回Ok(())，失败返回Status错误
    fn validate_message(&self, message: &Msg) -> Result<(), Status> {
        if message.send_id.is_empty() {
            return Err(Status::invalid_argument("发送者ID不能为空"));
        }
        if message.receiver_id.is_empty() {
            return Err(Status::invalid_argument("接收者ID不能为空"));
        }
        if message.content.is_empty() {
            return Err(Status::invalid_argument("消息内容不能为空"));
        }
        Ok(())
    }

    /// 将内部错误转换为gRPC状态
    /// 
    /// # 参数
    /// * `error` - 内部错误
    /// 
    /// # 返回值
    /// 返回对应的gRPC状态
    fn map_error_to_status(&self, error: Error) -> Status {
        match error {
            Error::BadRequest(msg) => Status::invalid_argument(msg),
            Error::NotFound(msg) => Status::not_found(msg),
            Error::Authentication(msg) => Status::unauthenticated(msg),
            Error::Internal(msg) => Status::internal(msg),
            Error::BroadCastError(msg) => Status::unavailable(msg),
            _ => Status::internal("内部服务器错误"),
        }
    }
}

// 注意: 以下实现需要等待proto文件生成后的类型
// #[async_trait]
// impl MessageStorageService for MessageStorageServiceImpl {
//     /// 保存单条消息到数据库和消息盒子
//     /// 
//     /// # 参数
//     /// * `request` - 保存消息请求
//     /// 
//     /// # 返回值
//     /// 返回保存结果
//     async fn save_message(
//         &self,
//         request: Request<SaveMessageRequest>,
//     ) -> Result<Response<SaveMessageResponse>, Status> {
//         let req = request.into_inner();
//         let message = req.message.ok_or_else(|| {
//             Status::invalid_argument("消息不能为空")
//         })?;

//         // 验证消息
//         self.validate_message(&message)?;

//         info!("保存单条消息: send_id={}, receiver_id={}", 
//               message.send_id, message.receiver_id);

//         // 保存到PostgreSQL (历史消息)
//         if req.need_to_history {
//             if let Err(e) = self.db_repo.msg.save_message(message.clone()).await {
//                 error!("保存消息到PostgreSQL失败: {:?}", e);
//                 return Ok(Response::new(SaveMessageResponse {
//                     success: false,
//                     error: format!("保存历史消息失败: {}", e),
//                 }));
//             }
//         }

//         // 保存到MongoDB (离线消息盒子)
//         if let Err(e) = self.msg_box_repo.save_message(&message).await {
//             error!("保存消息到MongoDB失败: {:?}", e);
//             return Ok(Response::new(SaveMessageResponse {
//                 success: false,
//                 error: format!("保存离线消息失败: {}", e),
//             }));
//         }

//         debug!("消息保存成功: server_id={}", message.server_id);
//         Ok(Response::new(SaveMessageResponse {
//             success: true,
//             error: String::new(),
//         }))
//     }

//     /// 保存群聊消息
//     /// 
//     /// # 参数
//     /// * `request` - 保存群聊消息请求
//     /// 
//     /// # 返回值
//     /// 返回保存结果
//     async fn save_group_message(
//         &self,
//         request: Request<SaveGroupMsgRequest>,
//     ) -> Result<Response<SaveMessageResponse>, Status> {
//         let req = request.into_inner();
//         let message = req.message.ok_or_else(|| {
//             Status::invalid_argument("消息不能为空")
//         })?;

//         // 验证消息
//         self.validate_message(&message)?;

//         info!("保存群聊消息: send_id={}, group_id={}, 成员数量={}", 
//               message.send_id, message.group_id, req.members.len());

//         // 保存到PostgreSQL (历史消息)
//         if req.need_to_history {
//             if let Err(e) = self.db_repo.msg.save_message(message.clone()).await {
//                 error!("保存群聊消息到PostgreSQL失败: {:?}", e);
//                 return Ok(Response::new(SaveMessageResponse {
//                     success: false,
//                     error: format!("保存群聊历史消息失败: {}", e),
//                 }));
//             }
//         }

//         // 批量更新成员序列号
//         let need_update_users: Vec<String> = req.members
//             .iter()
//             .filter(|m| m.need_update)
//             .map(|m| m.mem_id.clone())
//             .collect();

//         if !need_update_users.is_empty() {
//             if let Err(e) = self.db_repo.seq.save_max_seq_batch(&need_update_users).await {
//                 warn!("批量更新序列号失败: {:?}", e);
//             }
//         }

//         // 保存群聊消息到MongoDB
//         if let Err(e) = self.msg_box_repo.save_group_msg(message.clone(), req.members).await {
//             error!("保存群聊消息到MongoDB失败: {:?}", e);
//             return Ok(Response::new(SaveMessageResponse {
//                 success: false,
//                 error: format!("保存群聊离线消息失败: {}", e),
//             }));
//         }

//         debug!("群聊消息保存成功: server_id={}", message.server_id);
//         Ok(Response::new(SaveMessageResponse {
//             success: true,
//             error: String::new(),
//         }))
//     }

//     /// 获取用户消息(分页)
//     /// 
//     /// # 参数
//     /// * `request` - 获取消息请求
//     /// 
//     /// # 返回值
//     /// 返回消息列表
//     async fn get_messages(
//         &self,
//         request: Request<GetDbMsgRequest>,
//     ) -> Result<Response<GetMessagesResponse>, Status> {
//         let req = request.into_inner();

//         // 验证请求参数
//         if req.user_id.is_empty() {
//             return Err(Status::invalid_argument("用户ID不能为空"));
//         }
//         if req.start < 0 || req.end < 0 || req.start > req.end {
//             return Err(Status::invalid_argument("无效的序列号范围"));
//         }

//         info!("获取用户消息: user_id={}, start={}, end={}", 
//               req.user_id, req.start, req.end);

//         // 从MongoDB获取消息
//         let messages = self.msg_box_repo
//             .get_messages(&req.user_id, req.start, req.end)
//             .await
//             .map_err(|e| self.map_error_to_status(e))?;

//         let total_count = messages.len() as i32;
//         let has_more = total_count == (req.end - req.start + 1) as i32;

//         debug!("获取到{}条消息", total_count);
//         Ok(Response::new(GetMessagesResponse {
//             messages,
//             total_count,
//             has_more,
//         }))
//     }

//     /// 获取用户消息(包含发送和接收)
//     /// 
//     /// # 参数
//     /// * `request` - 获取消息请求
//     /// 
//     /// # 返回值
//     /// 返回消息列表
//     async fn get_db_messages(
//         &self,
//         request: Request<GetDbMessagesRequest>,
//     ) -> Result<Response<GetMessagesResponse>, Status> {
//         let req = request.into_inner();

//         // 验证请求参数
//         if req.user_id.is_empty() {
//             return Err(Status::invalid_argument("用户ID不能为空"));
//         }

//         info!("获取用户所有消息: user_id={}, send_range=({}, {}), rec_range=({}, {})", 
//               req.user_id, req.send_start, req.send_end, req.start, req.end);

//         // 从MongoDB获取复合消息
//         let messages = self.msg_box_repo
//             .get_msgs(&req.user_id, req.send_start, req.send_end, req.start, req.end)
//             .await
//             .map_err(|e| self.map_error_to_status(e))?;

//         let total_count = messages.len() as i32;
//         let has_more = false; // 这个接口通常获取全部数据

//         debug!("获取到{}条复合消息", total_count);
//         Ok(Response::new(GetMessagesResponse {
//             messages,
//             total_count,
//             has_more,
//         }))
//     }

//     /// 删除消息
//     /// 
//     /// # 参数
//     /// * `request` - 删除消息请求
//     /// 
//     /// # 返回值
//     /// 返回删除结果
//     async fn delete_messages(
//         &self,
//         request: Request<DelMsgRequest>,
//     ) -> Result<Response<DeleteMessageResponse>, Status> {
//         let req = request.into_inner();

//         if req.user_id.is_empty() {
//             return Err(Status::invalid_argument("用户ID不能为空"));
//         }
//         if req.msg_id.is_empty() {
//             return Err(Status::invalid_argument("消息ID列表不能为空"));
//         }

//         info!("删除用户消息: user_id={}, 消息数量={}", 
//               req.user_id, req.msg_id.len());

//         // 从MongoDB删除消息
//         match self.msg_box_repo.delete_messages(&req.user_id, req.msg_id.clone()).await {
//             Ok(_) => {
//                 debug!("成功删除{}条消息", req.msg_id.len());
//                 Ok(Response::new(DeleteMessageResponse {
//                     success: true,
//                     deleted_count: req.msg_id.len() as i32,
//                     error: String::new(),
//                 }))
//             }
//             Err(e) => {
//                 error!("删除消息失败: {:?}", e);
//                 Ok(Response::new(DeleteMessageResponse {
//                     success: false,
//                     deleted_count: 0,
//                     error: e.to_string(),
//                 }))
//             }
//         }
//     }

//     /// 标记消息为已读
//     /// 
//     /// # 参数
//     /// * `request` - 已读消息请求
//     /// 
//     /// # 返回值
//     /// 返回处理结果
//     async fn mark_messages_read(
//         &self,
//         request: Request<MsgReadReq>,
//     ) -> Result<Response<MsgReadResp>, Status> {
//         let req = request.into_inner();
//         let msg_read = req.msg_read.ok_or_else(|| {
//             Status::invalid_argument("已读消息信息不能为空")
//         })?;

//         if msg_read.user_id.is_empty() {
//             return Err(Status::invalid_argument("用户ID不能为空"));
//         }
//         if msg_read.msg_seq.is_empty() {
//             return Err(Status::invalid_argument("消息序列号列表不能为空"));
//         }

//         info!("标记消息为已读: user_id={}, 消息数量={}", 
//               msg_read.user_id, msg_read.msg_seq.len());

//         // 更新消息已读状态
//         self.msg_box_repo
//             .msg_read(&msg_read.user_id, &msg_read.msg_seq)
//             .await
//             .map_err(|e| self.map_error_to_status(e))?;

//         debug!("成功标记{}条消息为已读", msg_read.msg_seq.len());
//         Ok(Response::new(MsgReadResp {}))
//     }

//     /// 获取单条消息
//     /// 
//     /// # 参数
//     /// * `request` - 获取消息请求
//     /// 
//     /// # 返回值
//     /// 返回消息内容
//     async fn get_message(
//         &self,
//         request: Request<GetMessageRequest>,
//     ) -> Result<Response<GetMessageResponse>, Status> {
//         let req = request.into_inner();

//         if req.message_id.is_empty() {
//             return Err(Status::invalid_argument("消息ID不能为空"));
//         }

//         debug!("获取单条消息: message_id={}", req.message_id);

//         // 从MongoDB获取消息
//         match self.msg_box_repo.get_message(&req.message_id).await {
//             Ok(Some(message)) => {
//                 debug!("找到消息: {}", req.message_id);
//                 Ok(Response::new(GetMessageResponse {
//                     message: Some(message),
//                     found: true,
//                 }))
//             }
//             Ok(None) => {
//                 debug!("消息不存在: {}", req.message_id);
//                 Ok(Response::new(GetMessageResponse {
//                     message: None,
//                     found: false,
//                 }))
//             }
//             Err(e) => {
//                 error!("获取消息失败: {:?}", e);
//                 Err(self.map_error_to_status(e))
//             }
//         }
//     }

//     /// 流式获取消息
//     /// 
//     /// # 参数
//     /// * `request` - 获取消息请求
//     /// 
//     /// # 返回值
//     /// 返回消息流
//     async fn get_messages_stream(
//         &self,
//         request: Request<GetDbMsgRequest>,
//     ) -> Result<Response<Self::GetMessagesStreamStream>, Status> {
//         let req = request.into_inner();

//         // 验证请求参数
//         if req.user_id.is_empty() {
//             return Err(Status::invalid_argument("用户ID不能为空"));
//         }
//         if req.start < 0 || req.end < 0 || req.start > req.end {
//             return Err(Status::invalid_argument("无效的序列号范围"));
//         }

//         info!("流式获取用户消息: user_id={}, start={}, end={}", 
//               req.user_id, req.start, req.end);

//         // 获取消息流
//         let msg_receiver = self.msg_box_repo
//             .get_messages_stream(&req.user_id, req.start, req.end)
//             .await
//             .map_err(|e| self.map_error_to_status(e))?;

//         // 转换为gRPC流
//         let output_stream = tokio_stream::wrappers::ReceiverStream::new(msg_receiver)
//             .map(|result| match result {
//                 Ok(msg) => Ok(msg),
//                 Err(e) => Err(Status::internal(e.to_string())),
//             });

//         Ok(Response::new(Box::pin(output_stream)))
//     }

//     /// 保存用户最大序列号
//     /// 
//     /// # 参数
//     /// * `request` - 保存序列号请求
//     /// 
//     /// # 返回值
//     /// 返回保存结果
//     async fn save_max_seq(
//         &self,
//         request: Request<SaveMaxSeqRequest>,
//     ) -> Result<Response<SaveMaxSeqResponse>, Status> {
//         let req = request.into_inner();

//         if req.user_id.is_empty() {
//             return Err(Status::invalid_argument("用户ID不能为空"));
//         }

//         debug!("保存用户最大序列号: user_id={}", req.user_id);

//         // 更新用户最大序列号
//         match self.db_repo.seq.save_max_seq(&req.user_id).await {
//             Ok(new_seq) => {
//                 debug!("用户{}的新序列号: {}", req.user_id, new_seq);
//                 Ok(Response::new(SaveMaxSeqResponse {
//                     success: true,
//                     new_seq,
//                     error: String::new(),
//                 }))
//             }
//             Err(e) => {
//                 error!("保存最大序列号失败: {:?}", e);
//                 Ok(Response::new(SaveMaxSeqResponse {
//                     success: false,
//                     new_seq: 0,
//                     error: e.to_string(),
//                 }))
//             }
//         }
//     }

//     /// 健康检查
//     /// 
//     /// # 参数
//     /// * `request` - 健康检查请求
//     /// 
//     /// # 返回值
//     /// 返回服务状态
//     async fn health_check(
//         &self,
//         request: Request<HealthCheckRequest>,
//     ) -> Result<Response<HealthCheckResponse>, Status> {
//         let req = request.into_inner();
//         debug!("健康检查请求: service={}", req.service);

//         // 这里可以添加更复杂的健康检查逻辑
//         // 比如检查数据库连接、缓存连接等
        
//         Ok(Response::new(HealthCheckResponse {
//             status: HealthCheckResponse::ServingStatus::Serving as i32,
//         }))
//     }
// } 