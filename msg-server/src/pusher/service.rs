use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::error::Error;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, error, info};

use super::Pusher;
use common::config::AppConfig;
use common::message::msg_service_client::MsgServiceClient;
use common::message::{GroupMemSeq, Msg, SendGroupMsgRequest, SendMsgRequest};
use common::{service_register_center, ServiceRegister};

/// 消息推送服务的具体实现
/// 负责与多个WebSocket网关通信，将消息推送给在线客户端
#[derive(Debug)]
pub struct PusherService {
    // WebSocket RPC客户端列表，以网关的网络地址为键
    ws_rpc_list: Arc<DashMap<SocketAddr, MsgServiceClient<Channel>>>,
    // 服务发现客户端，用于查询WebSocket网关服务
    service_registry: Arc<dyn ServiceRegister>,
    // WebSocket服务名称
    sub_svr_name: String,
}

impl PusherService {
    /// 创建一个新的推送服务实例
    /// 初始化服务发现和WebSocket连接管理
    pub async fn new(config: &AppConfig) -> Self {
        // 获取WebSocket网关服务名称
        let sub_svr_name = config.rpc.ws.name.clone();
        // 创建WebSocket RPC客户端映射表
        let ws_rpc_list = Arc::new(DashMap::new());

        // 获取服务注册中心
        let service_registry = service_register_center(config);

        let service = Self {
            ws_rpc_list,
            service_registry,
            sub_svr_name,
        };

        // 初始化时尝试发现服务
        if let Err(err) = service.discover_services().await {
            error!("初始化服务发现失败: {:?}", err);
        }

        service
    }

    /// 发现WebSocket服务并建立连接
    async fn discover_services(&self) -> Result<(), Error> {
        // 查询WebSocket服务
        let service_urls = self
            .service_registry
            .discover_service(&self.sub_svr_name)
            .await
            .map_err(|e| Error::Internal(format!("查询WebSocket服务失败: {}", e)))?;

        if service_urls.is_empty() {
            debug!("未发现任何WebSocket服务实例");
            return Ok(());
        }

        info!("发现 {} 个WebSocket服务实例", service_urls.len());

        // 为每个服务URL创建RPC客户端
        for service_url in service_urls {
            if let Some(protocol_separator) = service_url.find("://") {
                let addr = &service_url[(protocol_separator + 3)..];

                // 解析为网络地址
                let socket = match addr.parse::<SocketAddr>() {
                    Ok(sa) => sa,
                    Err(err) => {
                        error!("解析服务地址失败: {:?}, addr: {}", err, addr);
                        continue;
                    }
                };

                // 连接WebSocket服务
                let endpoint = match Endpoint::from_shared(service_url.clone()) {
                    Ok(ep) => ep.connect_timeout(Duration::from_secs(5)),
                    Err(err) => {
                        error!("创建服务端点失败: {:?}, pg_url: {}", err, service_url);
                        continue;
                    }
                };

                // 创建RPC客户端
                match MsgServiceClient::connect(endpoint).await {
                    Ok(client) => {
                        self.ws_rpc_list.insert(socket, client);
                        info!("连接到WebSocket服务: {}", service_url);
                    }
                    Err(err) => {
                        error!("连接WebSocket服务失败: {:?}, pg_url: {}", err, service_url);
                    }
                };
            } else {
                error!("无效的服务URL格式: {}", service_url);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Pusher for PusherService {
    /// 推送单聊消息
    /// 将消息发送到所有WebSocket网关，由网关转发给目标用户
    async fn push_single_msg(&self, request: Msg) -> Result<(), Error> {
        debug!("推送单聊消息请求: {:?}", request);

        // 获取WebSocket RPC客户端列表
        let ws_rpc = self.ws_rpc_list.clone();

        // 如果列表为空，则尝试发现服务
        if ws_rpc.is_empty() {
            self.discover_services().await?;

            // 再次检查是否有可用服务
            if ws_rpc.is_empty() {
                return Err(Error::Internal("没有可用的WebSocket服务".to_string()));
            }
        }

        // 构建发送消息请求
        let request = SendMsgRequest {
            message: Some(request),
        };

        // 创建错误收集通道
        let (tx, mut rx) = mpsc::channel(ws_rpc.len());

        // 异步方式向所有WebSocket网关发送消息
        for v in ws_rpc.iter() {
            let tx = tx.clone();
            let service_id = *v.key();
            let mut v = v.clone();
            let request = request.clone();

            // 为每个网关创建单独的发送任务
            tokio::spawn(async move {
                if let Err(err) = v.send_msg_to_user(request).await {
                    if let Err(send_err) = tx.send((service_id, err)).await {
                        error!("发送错误通知失败: {:?}", send_err);
                    }
                }
            });
        }

        // 关闭发送端
        drop(tx);

        // 处理发送错误，从列表中移除失败的服务
        while let Some((service_id, err)) = rx.recv().await {
            ws_rpc.remove(&service_id);
            error!("向网关 {} 推送消息失败: {}", service_id, err);
        }

        Ok(())
    }

    /// 推送群聊消息
    /// 将消息发送到所有WebSocket网关，由网关转发给群成员
    async fn push_group_msg(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error> {
        debug!("推送群聊消息请求: {:?}, 成员: {:?}", msg, members);

        // 获取WebSocket RPC客户端列表
        let ws_rpc = self.ws_rpc_list.clone();

        // 如果列表为空，则尝试发现服务
        if ws_rpc.is_empty() {
            self.discover_services().await?;

            // 再次检查是否有可用服务
            if ws_rpc.is_empty() {
                return Err(Error::Internal("没有可用的WebSocket服务".to_string()));
            }
        }

        // 构建群聊消息请求
        let request = SendGroupMsgRequest {
            message: Some(msg),
            members,
        };

        // 创建错误收集通道
        let (tx, mut rx) = mpsc::channel(ws_rpc.len());

        // 异步方式向所有WebSocket网关发送群聊消息
        for v in ws_rpc.iter() {
            let tx = tx.clone();
            let service_id = *v.key();
            let mut v = v.clone();
            let request = request.clone();
            // 为每个网关创建单独的发送任务
            tokio::spawn(async move {
                if let Err(err) = v.send_group_msg_to_user(request).await {
                    if let Err(send_err) = tx.send((service_id, err)).await {
                        error!("发送错误通知失败: {:?}", send_err);
                    }
                }
            });
        }

        // 关闭发送端
        drop(tx);

        // 处理发送错误，从列表中移除失败的服务
        while let Some((service_id, err)) = rx.recv().await {
            ws_rpc.remove(&service_id);
            error!("向网关 {} 推送群消息失败: {}", service_id, err);
        }

        Ok(())
    }
}
