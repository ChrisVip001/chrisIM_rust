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

/// 心跳检测间隔时间（秒）
/// 
/// 用于定期向客户端发送ping消息，确认连接是否活跃。
/// 如果客户端在此时间内没有响应，连接将被认为已断开。
pub const HEART_BEAT_INTERVAL: u64 = 30;

/// 被踢下线的WebSocket关闭代码
/// 
/// 当用户在其他地方登录时，会向当前连接发送此关闭代码，
/// 通知客户端连接被强制关闭。
pub const KNOCK_OFF_CODE: u16 = 4001;

/// 未授权的WebSocket关闭代码
/// 
/// 当JWT令牌验证失败时，会向客户端发送此关闭代码，
/// 表示连接因为身份验证失败而被拒绝。
pub const UNAUTHORIZED_CODE: u16 = 4002;

/// WebSocket服务的应用状态
/// 
/// 包含连接管理器和JWT密钥，在所有WebSocket连接间共享。
/// 通过Axum的State机制传递给各个处理函数。
#[derive(Clone)]
pub struct AppState {
    /// 连接管理器
    /// 负责管理所有客户端连接和消息分发
    manager: Manager,
    
    /// JWT密钥
    /// 用于验证客户端连接时提供的JWT令牌
    jwt_secret: String,
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

/// WebSocket服务器实现
/// 
/// 负责处理WebSocket连接的建立、管理和消息处理。
/// 同时启动WebSocket服务器和gRPC服务器。
/// 
/// ## 主要功能
/// 
/// 1. **连接管理**: 处理WebSocket连接的建立和断开
/// 2. **身份验证**: 验证JWT令牌确保连接安全
/// 3. **消息处理**: 接收客户端消息并转发到处理队列
/// 4. **心跳检测**: 定期检查连接状态，清理无效连接
/// 5. **服务注册**: 向Consul注册服务供其他服务发现
pub struct WsServer;

impl WsServer {
    /// 向服务注册中心注册WebSocket服务
    /// 
    /// 将WebSocket服务注册到Consul，使其他服务能够发现并调用此服务。
    /// 注册信息包括服务名称、地址、端口和标签。
    /// 
    /// # 参数
    /// * `config` - 应用程序配置，包含服务注册信息
    /// 
    /// # 返回值
    /// * `Ok(String)` - 注册成功，返回服务ID
    /// * `Err(Error)` - 注册失败的错误信息
    async fn register_service(config: &AppConfig) -> Result<String, Error> {
        // 获取服务注册中心实例
        let service_register = service_register_center(config);

        // 构建服务注册信息
        let registration = Registration {
            // 服务唯一ID，包含服务名、主机和端口
            id: format!("{}-{}-{}", &config.websocket.name, &config.websocket.host, &config.websocket.port),
            // 服务名称，用于服务发现
            name: config.websocket.name.clone(),
            // 服务主机地址
            host: config.websocket.host.clone(),
            // 服务端口
            port: config.websocket.port,
            // 服务标签，用于分类和过滤
            tags: config.websocket.tags.clone(),
            // 健康检查配置（暂未使用）
            check: None,
        };

        // 向服务注册中心注册服务
        service_register.register(registration).await
    }

    /// 测试接口
    /// 
    /// 用于获取当前连接状态的调试接口。
    /// 返回所有已连接用户和平台的描述信息，便于开发和调试。
    /// 
    /// # 参数
    /// * `state` - 应用状态，包含连接管理器
    /// 
    /// # 返回值
    /// * `Ok(String)` - 连接状态的文本描述
    /// * `Err(Error)` - 获取状态失败的错误信息
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
    /// 这是服务的主要启动方法，执行以下步骤：
    /// 1. 创建消息通道和连接管理器
    /// 2. 配置Axum路由
    /// 3. 启动WebSocket服务器
    /// 4. 注册服务到Consul
    /// 5. 启动gRPC服务器
    /// 
    /// # 参数
    /// * `config` - 应用程序配置
    pub async fn start(config: Arc<AppConfig>) {
        // 创建消息通道，用于Manager和客户端之间的通信
        // 缓冲区大小为1024，可以根据实际负载调整
        let (tx, rx) = mpsc::channel(1024);
        
        // 初始化连接管理器
        let hub = Manager::new(tx, &config).await;
        let mut cloned_hub = hub.clone();
        
        // 在单独的任务中运行连接管理器
        // 这样可以并发处理消息而不阻塞WebSocket连接
        tokio::spawn(async move {
            cloned_hub.run(rx).await;
        });
        
        // 创建应用状态，包含管理器和JWT密钥
        let app_state = AppState {
            manager: hub.clone(),
            jwt_secret: config.gateway.auth.jwt.secret.clone(),
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
        let mut ws = tokio::spawn(async move {
            info!("WebSocket服务器已启动，监听地址: {}", addr);
            info!("WebSocket连接URL格式: /ws/{{user_id}}/conn/{{pointer_id}}/{{platform}}/{{token}}");
            info!("测试接口: /test");
            axum::serve(listener, router).await.unwrap();
        });

        // 向服务注册中心注册WebSocket服务
        Self::register_service(&config).await.unwrap();

        // 克隆配置用于RPC服务
        let config = config.clone();
        
        // 在独立任务中启动RPC服务
        let mut rpc = tokio::spawn(async move {
            // 启动RPC服务器，用于接收来自msg-server的消息推送请求
            MsgRpcService::start(hub, &config).await.expect("RPC服务器启动失败");
        });
        
        // 等待任一任务完成，并中止另一个任务
        // 正常情况下两个服务都会一直运行
        tokio::select! {
            _ = (&mut ws) => ws.abort(),
            _ = (&mut rpc) => rpc.abort(),
        }
    }

    /// 验证JWT令牌
    /// 
    /// 使用配置的JWT密钥验证客户端提供的令牌，
    /// 确保连接请求是授权的。
    /// 
    /// # 参数
    /// * `token` - 客户端提供的JWT令牌字符串
    /// * `jwt_secret` - JWT签名密钥
    /// 
    /// # 返回值
    /// * `Ok(())` - 令牌验证成功
    /// * `Err(Error)` - 令牌验证失败
    fn verify_token(token: String, jwt_secret: &String) -> Result<(), Error> {
        if let Err(err) = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(jwt_secret.as_bytes()),
            &Validation::default(),
        ) {
            return Err(Error::Authentication(format!(
                "JWT令牌验证失败: {}:/ws",
                err
            )));
        }
        Ok(())
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
        
        // 验证JWT令牌
        if let Err(err) = Self::verify_token(token, &app_state.jwt_secret) {
            warn!("JWT令牌验证失败: {:?}", err);
            
            // 如果验证失败，发送关闭消息并断开连接
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
        let mut watch_task = tokio::spawn(async move {
            if notify_receiver.recv().await.is_none() {
                info!("客户端 {} 被踢下线", pointer_id);
                
                // 向客户端发送踢下线信号
                if let Err(e) = shared_clone
                    .write()
                    .await
                    .send(Message::Close(Some(CloseFrame {
                        code: KNOCK_OFF_CODE,
                        reason: Utf8Bytes::from("账号在其他地方登录"),
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
                        // tracing::debug!("收到pong消息");
                    }
                    Message::Close(info) => {
                        // 客户端主动关闭连接
                        if let Some(info) = info {
                            warn!("客户端主动关闭连接: {}", info.reason);
                        }
                        break;
                    }
                    Message::Binary(b) => {
                        // 处理二进制格式的消息（主要消息格式）
                        let result = bincode::deserialize(&b);
                        if result.is_err() {
                            error!("二进制反序列化失败: {:?}；原始数据: {:?}", result.err(), b);
                            continue;
                        }
                        let msg: Msg = result.unwrap();
                        
                        // TODO: 需要根据消息类型判断local_id是否为空
                        // if msg.local_id.is_empty() {
                        //     warn!("收到空消息");
                        //     continue;
                        // }
                        
                        // 将消息广播到处理队列
                        if cloned_hub.broadcast(msg).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        
        // 标记是否需要注销连接
        let mut need_unregister = true;
        
        // 等待任一任务完成，并终止其他任务
        tokio::select! {
            _ = (&mut ping_task) => {
                // 心跳任务结束，通常是连接断开
                rec_task.abort(); 
                watch_task.abort();
            },
            _ = (&mut watch_task) => {
                // 踢下线任务结束，不需要注销（已被踢下线）
                need_unregister = false; 
                rec_task.abort(); 
                ping_task.abort();
            },
            _ = (&mut rec_task) => {
                // 接收任务结束，通常是客户端断开连接
                ping_task.abort(); 
                watch_task.abort();
            },
        }

        // 如果需要注销，从连接管理器中移除客户端
        if need_unregister {
            hub.unregister(user_id, platform).await;
        }
        
        tracing::debug!("客户端连接已断开，当前连接数: {}", hub.hub.iter().count());
    }
}
