use std::result::Result;

use tonic::transport::Server;
use tonic::{async_trait, Request, Response, Status};
use tracing::{debug, info};

use crate::manager::Manager;
use common::config::AppConfig;
use common::error::Error;
use common::grpc::LoggingInterceptor;
use common::message::msg_service_server::MsgServiceServer;
use common::message::{
    msg_service_server::MsgService, SendGroupMsgRequest, SendMsgRequest, SendMsgResponse,
};

/// 消息RPC服务实现
/// 
/// 提供gRPC接口供msg-server调用，用于将消息推送给在线用户。
/// 这是WebSocket网关对外提供的主要服务接口。
/// 
/// ## 服务功能
/// 
/// 1. **消息推送**: 接收来自msg-server的消息推送请求
/// 2. **单聊分发**: 将单聊消息推送给目标用户
/// 3. **群聊分发**: 将群聊消息推送给所有群成员
/// 4. **连接管理**: 通过Manager管理所有WebSocket连接
/// 
/// ## gRPC接口
/// 
/// - `send_message`: 通用消息发送接口
/// - `send_msg_to_user`: 单聊消息推送接口
/// - `send_group_msg_to_user`: 群聊消息推送接口
pub struct MsgRpcService {
    /// 连接管理器
    /// 负责管理所有WebSocket连接和消息分发
    manager: Manager,
}

impl MsgRpcService {
    /// 创建新的消息RPC服务实例
    /// 
    /// # 参数
    /// * `manager` - 连接管理器实例
    /// 
    /// # 返回值
    /// 返回配置好的MsgRpcService实例
    pub fn new(manager: Manager) -> Self {
        Self { manager }
    }

    /// 启动消息RPC服务
    /// 
    /// 执行以下初始化步骤：
    /// 1. 创建健康检查服务
    /// 2. 配置日志拦截器
    /// 3. 启动gRPC服务器
    /// 
    /// # 参数
    /// * `manager` - 连接管理器实例
    /// * `config` - 应用程序配置
    /// 
    /// # 返回值
    /// * `Ok(())` - 服务启动成功
    /// * `Err(Error)` - 服务启动失败
    pub async fn start(manager: Manager, config: &AppConfig) -> Result<(), Error> {
        info!("正在启动WebSocket RPC服务...");

        // 创建gRPC健康检查服务
        // 用于监控服务健康状态，支持Kubernetes等容器编排工具的健康检查
        let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
        
        // 设置服务为健康状态
        // 只要服务能启动就认为是健康的
        health_reporter
            .set_serving::<MsgServiceServer<MsgRpcService>>()
            .await;

        // 创建gRPC日志拦截器
        // 用于记录所有RPC请求和响应，便于调试和监控
        let logging_interceptor = LoggingInterceptor::new();

        // 创建消息RPC服务实例并包装为gRPC服务
        let service = Self::new(manager);
        let svc = MsgServiceServer::with_interceptor(service, logging_interceptor);
        
        info!(
            "WebSocket RPC服务已启动，监听地址: {}",
            config.rpc.ws.rpc_server_url()
        );

        // 启动gRPC服务器
        // 同时提供健康检查服务和消息推送服务
        Server::builder()
            .add_service(health_service) // 健康检查服务
            .add_service(svc)            // 消息推送服务
            .serve(config.rpc.ws.rpc_server_url().parse().unwrap())
            .await?;
        Ok(())
    }
}

/// 实现gRPC的MsgService trait
/// 提供消息推送的具体业务逻辑
#[async_trait]
impl MsgService for MsgRpcService {
    /// 发送消息（通用接口）
    /// 
    /// 这是一个通用的消息发送接口，将消息广播到处理队列。
    /// 主要用于内部消息流转和测试。
    /// 
    /// # 参数
    /// * `request` - gRPC请求，包含要发送的消息
    /// 
    /// # 返回值
    /// * `Ok(Response<SendMsgResponse>)` - 发送成功
    /// * `Err(Status)` - 发送失败，返回错误状态
    async fn send_message(
        &self,
        request: Request<SendMsgRequest>,
    ) -> Result<Response<SendMsgResponse>, Status> {
        debug!("收到消息发送请求: {:?}", request);
        
        // 从gRPC请求中提取消息内容
        let msg = request
            .into_inner()
            .message
            .ok_or(Status::invalid_argument("消息内容不能为空"))?;
            
        // 将消息广播到处理队列
        self.manager.broadcast(msg).await?;
        
        // 返回成功响应
        let response = Response::new(SendMsgResponse {});
        Ok(response)
    }

    /// 向用户发送单聊消息
    /// 
    /// 这是msg-server推送单聊消息的主要接口。
    /// 消息会被推送给指定用户的所有在线客户端。
    /// 
    /// # 参数
    /// * `request` - gRPC请求，包含要推送的单聊消息
    /// 
    /// # 返回值
    /// * `Ok(Response<SendMsgResponse>)` - 推送成功
    /// * `Err(Status)` - 推送失败，返回错误状态
    /// 
    /// # 推送逻辑
    /// 1. 验证消息内容
    /// 2. 查找目标用户的所有在线连接
    /// 3. 向所有连接推送消息
    /// 4. 如果用户离线，消息已在数据库中，用户上线后会拉取
    async fn send_msg_to_user(
        &self,
        request: Request<SendMsgRequest>,
    ) -> Result<Response<SendMsgResponse>, Status> {
        // 从gRPC请求中提取消息内容
        let msg = request
            .into_inner()
            .message
            .ok_or(Status::invalid_argument("消息内容不能为空"))?;
            
        debug!("向用户发送消息: {:?}", msg);
        
        // 通过连接管理器推送单聊消息
        self.manager.send_single_msg(&msg.receiver_id, &msg).await;
        
        // 返回成功响应
        let response = Response::new(SendMsgResponse {});
        Ok(response)
    }

    /// 向群组成员发送群聊消息
    /// 
    /// 这是msg-server推送群聊消息的主要接口。
    /// 消息会被推送给所有在线的群成员，每个成员收到带有自己序列号的消息副本。
    /// 
    /// # 参数
    /// * `request` - gRPC请求，包含群聊消息和成员列表
    /// 
    /// # 返回值
    /// * `Ok(Response<SendMsgResponse>)` - 推送成功
    /// * `Err(Status)` - 推送失败，返回错误状态
    /// 
    /// # 推送逻辑
    /// 1. 验证消息内容和成员列表
    /// 2. 为每个群成员分配正确的序列号
    /// 3. 向所有在线成员推送消息
    /// 4. 离线成员的消息已在数据库中，上线后会拉取
    async fn send_group_msg_to_user(
        &self,
        request: Request<SendGroupMsgRequest>,
    ) -> Result<Response<SendMsgResponse>, Status> {
        // 从gRPC请求中提取消息和成员信息
        let req = request.into_inner();
        let msg = req
            .message
            .ok_or(Status::invalid_argument("消息内容不能为空"))?;
        let members = req.members;
        
        // 通过连接管理器推送群聊消息
        self.manager.send_group(members, msg).await;
        
        // 返回成功响应
        let response = Response::new(SendMsgResponse {});
        Ok(response)
    }
}
