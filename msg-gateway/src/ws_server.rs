use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Utf8Bytes};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{
    extract::ws::{Message, WebSocket},
    Router,
};
use futures::{SinkExt, StreamExt};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use common::config::AppConfig;
use common::error::Error;
use common::message::{Msg, PlatformType};
use common::service_register_center::{service_register_center, Registration};
use crate::client::Client;
use crate::manager::Manager;
use crate::rpc::MsgRpcService;

// 心跳检测间隔时间，单位为秒
// 用于定期向客户端发送ping消息，确认连接是否活跃
pub const HEART_BEAT_INTERVAL: u64 = 30;
// 被踢下线的WebSocket关闭代码
pub const KNOCK_OFF_CODE: u16 = 4001;
// 未授权的WebSocket关闭代码
pub const UNAUTHORIZED_CODE: u16 = 4002;

/// WebSocket服务的应用状态
/// 包含连接管理器和JWT密钥
#[derive(Clone)]
pub struct AppState {
    // 连接管理器，负责管理所有客户端连接
    manager: Manager,
    // JWT密钥，用于验证客户端token
    jwt_secret: String,
}

/// JWT令牌的声明结构
#[derive(Serialize, Deserialize)]
pub struct Claims {
    // 用户标识
    pub sub: String,
    // 过期时间
    pub exp: u64,
    // 颁发时间
    pub iat: u64,
}

/// WebSocket服务器实现
pub struct WsServer;

impl WsServer {
    /// 向服务注册中心注册WebSocket服务
    /// 使其他服务能够发现并调用此服务
    async fn register_service(config: &AppConfig) -> Result<String, Error> {
        // 获取服务注册中心
        let service_register = service_register_center(config);

        // 构建服务注册信息
        let registration = Registration {
            id: format!("{}-{}-{}", &config.websocket.name, &config.websocket.host, &config.websocket.port),
            name: config.websocket.name.clone(),
            host: config.websocket.host.clone(),
            port: config.websocket.port,
            tags: config.websocket.tags.clone(),
            check: None,
        };

        // 注册服务
        service_register.register(registration).await
    }

    /// 测试接口，用于获取当前连接状态
    /// 返回所有已连接用户和平台的描述信息
    async fn test(State(state): State<AppState>) -> Result<String, Error> {
        let mut description = String::new();

        // 遍历所有连接，生成描述信息
        state.manager.hub.iter().for_each(|entry| {
            let user_id = entry.key();
            let platforms = entry.value();
            description.push_str(&format!("UserID: {}\n", user_id));
            platforms.iter().for_each(|platform_entry| {
                let platform_type = platform_entry.key();
                let client = platform_entry.value();
                description.push_str(&format!(
                    "  Platform: {:?}, PlatformID: {}\n",
                    platform_type, client.platform_id
                ));
            });
        });
        Ok(description)
    }

    /// 启动WebSocket服务器
    /// 初始化管理器、设置路由并启动服务
    pub async fn start(config: Arc<AppConfig>) {
        // 创建消息通道，用于Manager和客户端之间的通信
        let (tx, rx) = mpsc::channel(1024);
        // 初始化连接管理器
        let hub = Manager::new(tx, &config).await;
        let mut cloned_hub = hub.clone();
        // 在单独的任务中运行连接管理器
        tokio::spawn(async move {
            cloned_hub.run(rx).await;
        });
        // 创建应用状态
        let app_state = AppState {
            manager: hub.clone(),
            jwt_secret: config.gateway.auth.jwt.secret.clone(),
        };

        // 配置Axum路由
        let router = Router::new()
            .route(
                "/ws/{user_id}/conn/{pointer_id}/{platform}/{token}",
                get(Self::websocket_handler),
            )
            .route("/test", get(Self::test))
            .with_state(app_state);
        // 构建监听地址
        let addr = format!("{}:{}", config.websocket.host, config.websocket.port);

        // 启动TCP监听器
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        // 在独立任务中启动WebSocket服务器
        let mut ws = tokio::spawn(async move {
            info!("start websocket server on {}", addr);
            axum::serve(listener, router).await.unwrap();
        });

        // 向服务注册中心注册WebSocket服务
        Self::register_service(&config).await.unwrap();

        // 克隆配置用于RPC服务
        let config = config.clone();
        // 在独立任务中启动RPC服务
        let mut rpc = tokio::spawn(async move {
            // 启动RPC服务器，用于接收来自msg-server的消息
            MsgRpcService::start(hub, &config).await.expect("RPC server start error");
        });
        
        // 等待任一任务完成，并中止另一个任务
        tokio::select! {
            _ = (&mut ws) => ws.abort(),
            _ = (&mut rpc) => rpc.abort(),
        }
    }

    /// 验证JWT令牌
    /// 确保连接请求是授权的
    fn verify_token(token: String, jwt_secret: &String) -> Result<(), Error> {
        if let Err(err) = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(jwt_secret.as_bytes()),
            &Validation::default(),
        ) {
            return Err(Error::Authentication(format!(
                "verify token error: {}:{}",
                err, "/ws"
            )));
        }
        Ok(())
    }

    /// WebSocket连接处理器
    /// 从URL路径中提取参数并处理连接升级
    pub async fn websocket_handler(
        Path((user_id, pointer_id, platform, token)): Path<(String, String, i32, String)>,
        ws: WebSocketUpgrade,
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        // 将平台类型转换为枚举值
        let platform = PlatformType::try_from(platform).unwrap_or_default();
        // 处理WebSocket连接升级
        ws.on_upgrade(move |socket| {
            Self::websocket(user_id, pointer_id, token, platform, socket, state)
        })
    }

    /// 处理WebSocket连接
    /// 建立连接后的主要逻辑处理
    pub async fn websocket(
        user_id: String,
        pointer_id: String,
        token: String,
        platform: PlatformType,
        ws: WebSocket,
        app_state: AppState,
    ) {
        tracing::info!(
            "客户端 {} 已连接，用户ID: {}",
            user_id.clone(),
            pointer_id.clone()
        );
        // 将WebSocket分为发送和接收两部分
        let (mut ws_tx, mut ws_rx) = ws.split();
        
        // 验证令牌
        if let Err(err) = Self::verify_token(token, &app_state.jwt_secret) {
            warn!("验证令牌错误: {:?}", err);
            // 如果验证失败，发送关闭消息
            if let Err(e) = ws_tx
                .send(Message::Close(Some(CloseFrame {
                    code: UNAUTHORIZED_CODE,
                    reason: Utf8Bytes::from("未授权连接"),
                })))
                .await
            {
                error!("发送验证失败消息给客户端时出错: {}", e);
            }
            return;
        }
        
        // 创建共享的发送通道
        let shared_tx = Arc::new(RwLock::new(ws_tx));
        // 创建通知通道，用于关闭连接
        let (notify_sender, mut notify_receiver) = tokio::sync::mpsc::channel(1);
        let mut hub = app_state.manager.clone();
        
        // 创建客户端对象
        let client = Client {
            user_id: user_id.clone(),
            platform_id: pointer_id.clone(),
            sender: shared_tx.clone(),
            platform,
            notify_sender,
        };
        
        // 向连接管理器注册客户端
        hub.register(user_id.clone(), client).await;

        // 发送心跳消息给客户端的任务
        let cloned_tx = shared_tx.clone();
        let mut ping_task = tokio::spawn(async move {
            loop {
                if let Err(e) = cloned_tx
                    .write()
                    .await
                    .send(Message::Ping(Default::default()))
                    .await
                {
                    error!("send ping error：{:?}", e);
                    // break this task, it will end this conn
                    break;
                }
                tokio::time::sleep(Duration::from_secs(HEART_BEAT_INTERVAL)).await;
            }
        });

        let shared_clone = shared_tx.clone();
        // watch knock off signal
        let mut watch_task = tokio::spawn(async move {
            if notify_receiver.recv().await.is_none() {
                info!("client {} knock off", pointer_id);
                // send knock off signal to ws server
                if let Err(e) = shared_clone
                    .write()
                    .await
                    .send(Message::Close(Some(CloseFrame {
                        code: KNOCK_OFF_CODE,
                        reason: Utf8Bytes::from("knock off"),
                    })))
                    .await
                {
                    error!("send knock off signal to client error: {}", e);
                }
            }
        });

        // spawn a new task to receive message
        let cloned_hub = hub.clone();
        let shared_tx = shared_tx.clone();
        // receive message from client
        let mut rec_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                // 处理消息
                match msg {
                    Message::Text(text) => {
                        let result = serde_json::from_str(&text);
                        if result.is_err() {
                            error!("deserialize error: {:?}； source: {text}", result.err());
                            continue;
                        }

                        if cloned_hub.broadcast(result.unwrap()).await.is_err() {
                            // if broadcast not available, close the connection
                            break;
                        }
                    }
                    Message::Ping(_) => {
                        if let Err(e) = shared_tx
                            .write()
                            .await
                            .send(Message::Pong(Default::default()))
                            .await
                        {
                            error!("reply ping error : {:?}", e);
                            break;
                        }
                    }
                    Message::Pong(_) => {
                        // tracing::debug!("received pong message");
                    }
                    Message::Close(info) => {
                        if let Some(info) = info {
                            warn!("client closed {}", info.reason);
                        }
                        break;
                    }
                    Message::Binary(b) => {
                        let result = bincode::deserialize(&b);
                        if result.is_err() {
                            error!("deserialize error: {:?}； source: {:?}", result.err(), b);
                            continue;
                        }
                        let msg: Msg = result.unwrap();
                        // todo need to judge the local id is empty by message type
                        // if msg.local_id.is_empty() {
                        //     warn!("receive empty message");
                        //     continue;
                        // }
                        if cloned_hub.broadcast(msg).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut need_unregister = true;
        tokio::select! {
            _ = (&mut ping_task) => {rec_task.abort(); watch_task.abort();},
            _ = (&mut watch_task) => {need_unregister = false; rec_task.abort(); ping_task.abort();},
            _ = (&mut rec_task) => {ping_task.abort(); watch_task.abort();},
        }

        // lost the connection, remove the client from hub
        if need_unregister {
            hub.unregister(user_id, platform).await;
        }
        tracing::debug!("client thread exit {}", hub.hub.iter().count());
    }
}
