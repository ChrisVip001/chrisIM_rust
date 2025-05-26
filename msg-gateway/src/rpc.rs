use std::result::Result;

use tonic::transport::Server;
use tonic::{async_trait, Request, Response, Status};
use tracing::{debug, info};
use tonic_health::server::HealthReporter;

use crate::manager::Manager;
use common::config::{AppConfig, Component};
use common::error::Error;
use common::grpc::LoggingInterceptor;
use common::message::msg_service_server::MsgServiceServer;
use common::message::{
    msg_service_server::MsgService, SendGroupMsgRequest, SendMsgRequest, SendMsgResponse,
};

pub struct MsgRpcService {
    manager: Manager,
}

impl MsgRpcService {
    pub fn new(manager: Manager) -> Self {
        Self { manager }
    }

    pub async fn start(manager: Manager, config: &AppConfig) -> Result<(), Error> {
        // 创建并注册到Consul
        common::grpc_client::base::register_service(config, Component::MessageGateway)
            .await
            .expect("服务注册失败");

        info!("<ws> rpc service register to service register center");

        // 创建gRPC健康检查服务
        let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
        
        // 设置服务健康状态
        health_reporter
            .set_serving::<MsgServiceServer<MsgRpcService>>()
            .await;

        // 启动一个后台任务来定期检查WebSocket连接管理器状态并更新健康状态
        let manager_health = manager.clone();
        let mut health_reporter_clone = health_reporter.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                
                // 检查WebSocket连接管理器状态 - 通过hub长度获取连接数
                let connection_count = manager_health.hub.len();
                
                // 如果管理器正常工作（能够获取连接数），则认为服务健康
                // 这里我们简单地检查管理器是否响应，实际项目中可以添加更复杂的检查
                let _ = health_reporter_clone
                    .set_serving::<MsgServiceServer<MsgRpcService>>()
                    .await;
                
                debug!("WebSocket连接数: {}", connection_count);
            }
        });

        info!("<ws> rpc service health check started");

        // 创建日志拦截器
        let logging_interceptor = LoggingInterceptor::new();

        let service = Self::new(manager);
        let svc = MsgServiceServer::with_interceptor(service, logging_interceptor);
        info!(
            "<ws> rpc service started at {}",
            config.rpc.ws.rpc_server_url()
        );

        Server::builder()
            .add_service(health_service) // 添加健康检查服务
            .add_service(svc)
            .serve(config.rpc.ws.rpc_server_url().parse().unwrap())
            .await?;
        Ok(())
    }
}

#[async_trait]
impl MsgService for MsgRpcService {
    async fn send_message(
        &self,
        request: Request<SendMsgRequest>,
    ) -> Result<Response<SendMsgResponse>, Status> {
        debug!("Got a request: {:?}", request);
        let msg = request
            .into_inner()
            .message
            .ok_or(Status::invalid_argument("message is empty"))?;
        self.manager.broadcast(msg).await?;
        let response = Response::new(SendMsgResponse {});
        Ok(response)
    }

    /// Send message to user
    /// pusher will procedure this to send message to user
    async fn send_msg_to_user(
        &self,
        request: Request<SendMsgRequest>,
    ) -> Result<Response<SendMsgResponse>, Status> {
        let msg = request
            .into_inner()
            .message
            .ok_or(Status::invalid_argument("message is empty"))?;
        debug!("send message to user: {:?}", msg);
        self.manager.send_single_msg(&msg.receiver_id, &msg).await;
        let response = Response::new(SendMsgResponse {});
        Ok(response)
    }

    async fn send_group_msg_to_user(
        &self,
        request: Request<SendGroupMsgRequest>,
    ) -> Result<Response<SendMsgResponse>, Status> {
        let req = request.into_inner();
        let msg = req
            .message
            .ok_or(Status::invalid_argument("message is empty"))?;
        let members = req.members;
        self.manager.send_group(members, msg).await;
        let response = Response::new(SendMsgResponse {});
        Ok(response)
    }
}
