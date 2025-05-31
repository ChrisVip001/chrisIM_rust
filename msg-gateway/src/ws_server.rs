use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use common::auth::verify_token_simple;
use common::config::AppConfig;
use common::error::Error;
use common::message::PlatformType;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use crate::client::Client;
use crate::manager::Manager;
use crate::rpc::MsgRpcService;

/// 心跳间隔时间（秒）
///
/// WebSocket连接的心跳检测间隔，用于检测连接是否仍然活跃。
/// 客户端需要在此间隔内响应ping消息，否则连接可能被认为已断开。
pub const HEART_BEAT_INTERVAL: u64 = 30;

/// 踢下线状态码
///
/// 当用户在其他地方登录时，向当前连接发送此状态码以踢下线。
/// 客户端收到此状态码应该理解为账号在其他地方登录。
pub const KNOCK_OFF_CODE: u16 = 4001;

/// 未授权状态码
///
/// 当JWT令牌验证失败时，向客户端发送此状态码。
/// 客户端收到此状态码应该重新进行身份验证。
pub const UNAUTHORIZED_CODE: u16 = 4002;

/// 应用状态
///
/// 包含WebSocket服务器运行所需的全局状态信息
#[derive(Clone)]
pub struct AppState {
    /// 连接管理器
    /// 负责管理所有客户端连接和消息分发
    manager: Manager,

    /// JWT配置
    /// 用于验证客户端连接时提供的JWT令牌
    jwt_config: common::configs::auth_config::JwtConfig,
}

/// JWT令牌的声明结构
///
/// 定义了JWT令牌中包含的标准字段，用于用户身份验证。
#[derive(Serialize, Deserialize)]
pub struct Claims {
    /// 用户标识（Subject）
    /// JWT标准字段，表示令牌的主体用户
    pub sub: String,

    /// 过期时间（Expiration Time）
    /// JWT标准字段，Unix时间戳格式
    pub exp: u64,

    /// 颁发时间（Issued At）
    /// JWT标准字段，Unix时间戳格式
    pub iat: u64,
}

/// WebSocket服务器
///
/// 负责处理WebSocket连接、消息路由和客户端管理
pub struct WsServer;

impl WsServer {
    /// 测试接口
    ///
    /// 提供一个简单的HTTP接口用于测试服务状态和查看连接信息
    async fn test(State(state): State<AppState>) -> Result<String, Error> {
        let mut description = String::new();

        // 遍历所有连接，生成描述信息
        state.manager.hub.iter().for_each(|entry| {
            let user_id = entry.key();
            let platforms = entry.value();
            description.push_str(&format!("用户ID: {}\n", user_id));

            // 遍历该用户的所有平台连接
            platforms.iter().for_each(|platform_entry| {
                let platform_type = platform_entry.key();
                let client = platform_entry.value();
                description.push_str(&format!(
                    "  平台: {:?}, 平台ID: {}\n",
                    platform_type, client.platform_id
                ));
            });
        });
        Ok(description)
    }

    /// 启动WebSocket服务器
    ///
    /// 这是WebSocket服务器的主入口点，负责：
    /// 1. 初始化连接管理器
    /// 2. 启动WebSocket HTTP服务器
    /// 3. 启动gRPC服务器
    /// 4. 注册服务到服务发现中心
    ///
    /// # 参数
    /// * `config` - 应用配置
    /// * `shutdown_rx` - 优雅关闭信号接收器
    pub async fn start(config: Arc<AppConfig>, mut shutdown_rx: tokio::sync::oneshot::Receiver<()>) {
        // 创建连接管理器和消息通道
        let (tx, rx) = mpsc::channel(1024);
        let hub = Manager::new(tx, &config).await;
        let mut cloned_hub = hub.clone();

        // 在单独的任务中运行连接管理器
        // 这样可以并发处理消息而不阻塞WebSocket连接
        tokio::spawn(async move {
            cloned_hub.run(rx).await;
        });

        // 创建应用状态，包含管理器和JWT配置
        let app_state = AppState {
            manager: hub.clone(),
            jwt_config: config.gateway.auth.jwt.clone(),
        };

        // 配置Axum路由
        let router = Router::new()
            // WebSocket连接路由，包含所有必要的路径参数
            .route(
                "/ws/{user_id}/conn/{pointer_id}/{platform}/{token}",
                get(Self::websocket_handler),
            )
            // 测试路由，用于查看连接状态
            .route("/test", get(Self::test))
            // 设置应用状态，所有路由都可以访问
            .with_state(app_state);

        // 构建监听地址
        let addr = format!("{}:{}", config.websocket.host, config.websocket.port);

        // 启动TCP监听器
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

        // 在独立任务中启动WebSocket服务器
        let mut ws_task = tokio::spawn(async move {
            info!("WebSocket服务器已启动，监听地址: {}", addr);
            info!(
                "WebSocket连接URL格式: /ws/{{user_id}}/conn/{{pointer_id}}/{{platform}}/{{token}}"
            );
            info!("测试接口: /test");
            axum::serve(listener, router).await.unwrap();
        });

        // 在独立任务中启动RPC服务
        let mut rpc_task = tokio::spawn(async move {
            // 启动RPC服务器，用于接收来自msg-server的消息推送请求
            MsgRpcService::start(hub, &config)
                .await
                .expect("RPC服务器启动失败");
        });

        // 等待关闭信号或任一任务完成
        tokio::select! {
            _ = (&mut shutdown_rx) => {
                info!("收到关闭信号，正在停止WebSocket网关服务...");
                ws_task.abort();
                rpc_task.abort();
            },
            _ = (&mut ws_task) => {
                error!("WebSocket服务器意外退出");
                rpc_task.abort();
            },
            _ = (&mut rpc_task) => {
                error!("RPC服务器意外退出");
                ws_task.abort();
            },
        }
    }

    /// 验证JWT令牌
    ///
    /// 使用统一的JWT验证逻辑，确保与api-gateway的验证规则完全一致。
    /// 这解决了之前两个服务验证逻辑不一致的问题。
    ///
    /// # 参数
    /// * `token` - 客户端提供的JWT令牌字符串
    /// * `jwt_config` - JWT配置信息
    ///
    /// # 返回值
    /// * `Ok(())` - 令牌验证成功
    /// * `Err(Error)` - 令牌验证失败
    fn verify_token(
        token: &str,
        jwt_config: &common::configs::auth_config::JwtConfig,
    ) -> Result<(), Error> {
        // 使用common模块中的统一验证函数
        // 这确保了与api-gateway完全相同的验证逻辑
        verify_token_simple(token, jwt_config)
            .map_err(|e| Error::Authentication(format!("JWT令牌验证失败: {}", e)))
    }

    /// WebSocket连接处理器
    ///
    /// 从URL路径中提取连接参数并处理WebSocket连接升级。
    /// 这是Axum路由的处理函数，负责将HTTP请求升级为WebSocket连接。
    ///
    /// # 参数
    /// * `Path((user_id, pointer_id, platform, token))` - 从URL路径提取的参数
    /// * `ws` - WebSocket升级请求
    /// * `state` - 应用状态
    ///
    /// # 返回值
    /// 返回WebSocket升级响应
    pub async fn websocket_handler(
        Path((user_id, pointer_id, platform, token)): Path<(String, String, i32, String)>,
        ws: WebSocketUpgrade,
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        // 将平台类型从整数转换为枚举值
        let platform = PlatformType::try_from(platform).unwrap_or_default();

        // 处理WebSocket连接升级
        // 升级成功后会调用websocket函数处理连接
        ws.on_upgrade(move |socket| {
            Self::websocket(user_id, pointer_id, token, platform, socket, state)
        })
    }

    /// 处理WebSocket连接
    ///
    /// 建立WebSocket连接后的主要逻辑处理，包括：
    /// 1. 验证JWT令牌
    /// 2. 注册客户端连接
    /// 3. 启动心跳检测
    /// 4. 处理消息收发
    /// 5. 管理连接生命周期
    ///
    /// # 参数
    /// * `user_id` - 用户ID
    /// * `pointer_id` - 客户端唯一标识
    /// * `token` - JWT令牌
    /// * `platform` - 平台类型
    /// * `ws` - WebSocket连接
    /// * `app_state` - 应用状态
    pub async fn websocket(
        user_id: String,
        pointer_id: String,
        token: String,
        platform: PlatformType,
        ws: WebSocket,
        app_state: AppState,
    ) {
        tracing::info!(
            "客户端连接建立: 用户ID={}, 平台ID={}, 平台类型={:?}",
            user_id.clone(),
            pointer_id.clone(),
            platform
        );

        // 将WebSocket分为发送和接收两部分
        // 这样可以在不同的任务中并发处理发送和接收
        let (mut ws_tx, mut ws_rx) = ws.split();

        // 验证JWT令牌 - 使用统一的验证逻辑
        if let Err(err) = Self::verify_token(&token, &app_state.jwt_config) {
            warn!("JWT令牌验证失败: {:?}", err);

            // 如果验证失败，发送关闭消息并断开连接
            if let Err(e) = ws_tx
                .send(Message::Close(Some(CloseFrame {
                    code: UNAUTHORIZED_CODE,
                    reason: "未授权连接".into(),
                })))
                .await
            {
                error!("发送验证失败消息给客户端时出错: {}", e);
            }
            return;
        }

        // 创建共享的发送通道，支持多线程安全访问
        let shared_tx = Arc::new(RwLock::new(ws_tx));

        // 创建通知通道，用于踢下线等控制信号
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

        // 启动心跳检测任务
        // 定期向客户端发送ping消息，检查连接是否活跃
        let cloned_tx = shared_tx.clone();
        let mut ping_task = tokio::spawn(async move {
            loop {
                if let Err(e) = cloned_tx
                    .write()
                    .await
                    .send(Message::Ping(Default::default()))
                    .await
                {
                    error!("发送心跳消息失败：{:?}", e);
                    // 发送失败表示连接已断开，退出心跳任务
                    break;
                }
                // 等待下一次心跳间隔
                tokio::time::sleep(Duration::from_secs(HEART_BEAT_INTERVAL)).await;
            }
        });

        // 启动踢下线监听任务
        // 监听来自其他地方的踢下线信号
        let shared_clone = shared_tx.clone();
        let pointer_id_clone = pointer_id.clone();
        let mut watch_task = tokio::spawn(async move {
            if notify_receiver.recv().await.is_none() {
                info!("客户端 {} 被踢下线", pointer_id_clone);

                // 向客户端发送踢下线信号
                if let Err(e) = shared_clone
                    .write()
                    .await
                    .send(Message::Close(Some(CloseFrame {
                        code: KNOCK_OFF_CODE,
                        reason: "账号在其他地方登录".into(),
                    })))
                    .await
                {
                    error!("发送踢下线信号给客户端失败: {}", e);
                }
            }
        });

        // 启动消息接收任务
        // 处理来自客户端的所有消息
        let cloned_hub = hub.clone();
        let shared_tx = shared_tx.clone();
        let mut rec_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                // 根据消息类型进行不同处理
                match msg {
                    Message::Text(text) => {
                        // 处理JSON格式的文本消息
                        let result = serde_json::from_str(&text);
                        if result.is_err() {
                            error!("JSON反序列化失败: {:?}；原始内容: {}", result.err(), text);
                            continue;
                        }

                        // 将消息广播到处理队列
                        if cloned_hub.broadcast(result.unwrap()).await.is_err() {
                            // 如果广播失败，说明服务不可用，关闭连接
                            break;
                        }
                    }
                    Message::Ping(_) => {
                        // 响应客户端的ping消息
                        if let Err(e) = shared_tx
                            .write()
                            .await
                            .send(Message::Pong(Default::default()))
                            .await
                        {
                            error!("回复ping消息失败: {:?}", e);
                            break;
                        }
                    }
                    Message::Pong(_) => {
                        // 收到客户端的pong响应，连接正常
                        info!("收到客户端pong响应，连接正常");
                    }
                    Message::Close(_) => {
                        // 客户端主动关闭连接
                        info!("客户端主动关闭连接");
                        break;
                    }
                    _ => {
                        // 其他类型的消息暂不处理
                        warn!("收到未处理的消息类型: {:?}", msg);
                    }
                }
            }
        });

        // 等待任一任务完成，然后清理资源
        tokio::select! {
            _ = (&mut ping_task) => {
                info!("心跳任务结束");
                rec_task.abort();
                watch_task.abort();
            }
            _ = (&mut rec_task) => {
                info!("消息接收任务结束");
                ping_task.abort();
                watch_task.abort();
            }
            _ = (&mut watch_task) => {
                info!("踢下线监听任务结束");
                ping_task.abort();
                rec_task.abort();
            }
        }

        // 从连接管理器中注销客户端
        hub.unregister(user_id.clone(), platform).await;

        info!(
            "客户端连接已断开: 用户ID={}, 平台ID={}",
            user_id, pointer_id
        );
    }
}
