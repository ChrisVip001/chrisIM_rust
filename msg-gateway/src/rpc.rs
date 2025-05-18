use std::result::Result;

use tonic::transport::Server;
use tonic::{async_trait, Request, Response, Status};
use tracing::{debug, info};

use crate::manager::Manager;
use common::config::{AppConfig, Component};
use common::error::Error;
use common::grpc::LoggingInterceptor;
use common::message::msg_service_server::MsgServiceServer;
use common::message::{
    msg_service_server::MsgService, SendGroupMsgRequest, SendMsgRequest, SendMsgResponse,
};
use common::service_registry::ServiceRegistry;
use tonic_health::server::{Health, HealthServer};

pub struct MsgRpcService {
    manager: Manager,
}

impl MsgRpcService {
    pub fn new(manager: Manager) -> Self {
        Self { manager }
    }

    pub async fn start(manager: Manager, config: &AppConfig) -> Result<(), Error> {
        // register service to service register center
        // 创建并注册到Consul
        let service_registry = ServiceRegistry::from_env();
        let service_id = service_registry
            .register_service(
                "msg-gateway",
                &config.server.host,
                config.server.port as u32, // 显式转换为u32类型
                vec!["auth".to_string(), "api".to_string()],
                "/health",
                "15s",
            )
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;
        info!("<ws> rpc service register to service register center");

        // open health check
        let health_service = HealthServer::new(Health::default());
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
            .add_service(health_service)
            .add_service(svc)
            .serve(config.rpc.ws.rpc_server_url().parse().unwrap())
            .await
            .unwrap();
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
