use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::error::Error;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tonic::transport::{Channel, Endpoint};
use tower::discover::Change;
use tracing::{debug, error};

use common::config::AppConfig;
use common::message::msg_service_client::MsgServiceClient;
use common::message::{GroupMemSeq, Msg, SendGroupMsgRequest, SendMsgRequest};

use super::Pusher;

/// 消息推送服务的具体实现
/// 负责与多个WebSocket网关通信，将消息推送给在线客户端
#[derive(Debug)]
pub struct PusherService {
    // WebSocket RPC客户端列表，以网关的网络地址为键
    ws_rpc_list: Arc<DashMap<SocketAddr, MsgServiceClient<Channel>>>,
    // 服务中心客户端，用于查询WebSocket网关服务
    service_center: ServiceClient,
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
        let cloned_list = ws_rpc_list.clone();
        // 创建服务变更通知通道
        let (tx, mut rx) = mpsc::channel::<Change<SocketAddr, Endpoint>>(100);

        // 启动后台任务，处理服务变更通知
        tokio::spawn(async move {
            while let Some(change) = rx.recv().await {
                debug!("接收到服务变更: {:?}", change);
                match change {
                    // 添加新的WebSocket服务
                    Change::Insert(service_id, client) => {
                        match MsgServiceClient::connect(client).await {
                            Ok(client) => {
                                cloned_list.insert(service_id, client);
                            }
                            Err(err) => {
                                error!("连接WebSocket服务失败: {:?}", err);
                            }
                        };
                    }
                    // 移除已下线的WebSocket服务
                    Change::Remove(service_id) => {
                        cloned_list.remove(&service_id);
                    }
                }
            }
        });

        // 获取服务发现通道，用于接收服务变更通知
        utils::get_chan_(config, sub_svr_name.clone(), tx)
            .await
            .unwrap();

        // 创建服务中心客户端
        let service_center = ServiceClient::builder()
            .server_host(config.service_center.host.clone())
            .server_port(config.service_center.port)
            .connect_timeout(Duration::from_millis(config.service_center.timeout))
            .build()
            .await
            .unwrap();
            
        Self {
            ws_rpc_list,
            service_center,
            sub_svr_name,
        }
    }

    /// 处理WebSocket网关服务列表
    /// 为每个服务创建RPC客户端连接
    pub async fn handle_sub_services(&self, services: Vec<Service>) {
        for service in services {
            // 构建服务地址
            let addr = format!("{}:{}", service.address, service.port);
            // 解析为网络地址
            let socket: SocketAddr = match addr.parse() {
                Ok(sa) => sa,
                Err(err) => {
                    error!("解析服务地址失败: {:?}", err);
                    continue;
                }
            };
            // 构建完整地址，包含协议
            let addr = format!("{}://{}", service.scheme, addr);
            // 连接WebSocket服务
            let endpoint = match Endpoint::from_shared(addr) {
                Ok(ep) => ep.connect_timeout(Duration::from_secs(5)),
                Err(err) => {
                    error!("创建服务端点失败: {:?}", err);
                    continue;
                }
            };
            // 创建RPC客户端
            let ws = match MsgServiceClient::connect(endpoint).await {
                Ok(client) => client,
                Err(err) => {
                    error!("连接WebSocket服务失败: {:?}", err);
                    continue;
                }
            };
            // 添加到客户端列表
            self.ws_rpc_list.insert(socket, ws);
        }
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
        // 如果列表为空，则从服务中心查询WebSocket服务
        if ws_rpc.is_empty() {
            let mut client = self.service_center.clone();
            let list = client
                .query_with_name(self.sub_svr_name.clone())
                .await
                .map_err(|e| Error::Internal(e.to_string()))?;
            // 为查询到的服务创建RPC客户端
            self.handle_sub_services(list).await;
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
                    tx.send((service_id, err)).await.unwrap();
                };
            });
        }

        // 关闭发送端
        drop(tx);

        // 处理发送错误，从列表中移除失败的服务
        // TODO: 需要更新客户端列表并处理错误
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
        // 如果列表为空，则从服务中心查询WebSocket服务
        if ws_rpc.is_empty() {
            let mut client = self.service_center.clone();
            let list = client
                .query_with_name(self.sub_svr_name.clone())
                .await
                .map_err(|e| Error::Internal(e.to_string()))?;
            // 为查询到的服务创建RPC客户端
            self.handle_sub_services(list).await;
        }

        // 构建群聊消息请求
        let request = SendGroupMsgRequest {
            message: Some(msg),
            members,
        };
        // 创建结果收集通道
        let (tx, mut rx) = mpsc::channel(ws_rpc.len());
        
        // 异步方式向所有WebSocket网关发送群聊消息
        for v in ws_rpc.iter() {
            let tx = tx.clone();
            let service_id = *v.key();
            let mut v = v.clone();
            let request = request.clone();
            // 为每个网关创建单独的发送任务
            tokio::spawn(async move {
                match v.send_group_msg_to_user(request).await {
                    Ok(_) => {
                        tx.send(Ok(())).await.unwrap();
                    }
                    Err(err) => {
                        tx.send(Err((service_id, err))).await.unwrap();
                    }
                };
            });
        }
        // 关闭发送端
        drop(tx);
        
        // 处理发送错误，从列表中移除失败的服务
        // TODO: 需要更新客户端列表
        while let Some(Err((service_id, err))) = rx.recv().await {
            ws_rpc.remove(&service_id);
            error!("向网关 {} 推送群聊消息失败: {}", service_id, err);
        }
        Ok(())
    }
}
