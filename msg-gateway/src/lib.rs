/// msg-gateway WebSocket网关服务库
/// 
/// 这是即时通讯系统的WebSocket网关服务，负责：
/// 
/// ## 核心功能
/// 
/// ### `client` 模块 - 客户端连接管理
/// - 封装WebSocket客户端连接
/// - 提供消息发送接口（文本和二进制）
/// - 管理客户端平台信息和用户标识
/// 
/// ### `manager` 模块 - 连接管理器
/// - 管理所有客户端连接的生命周期
/// - 处理消息路由和分发
/// - 支持单聊和群聊消息推送
/// - 与msg-server通过gRPC通信
/// 
/// ### `rpc` 模块 - gRPC服务接口
/// - 提供MsgService的gRPC实现
/// - 接收来自msg-server的消息推送请求
/// - 支持单聊和群聊消息的分发
/// 
/// ### `ws_server` 模块 - WebSocket服务器
/// - 处理WebSocket连接建立和升级
/// - JWT令牌验证和用户认证
/// - 心跳检测和连接保活
/// - 服务注册和发现
/// 
/// ## 工作流程
/// 
/// ```text
/// 客户端WebSocket连接 -> WsServer -> Manager -> gRPC接口 -> msg-server
///                                    ↓
/// 消息推送 <- Manager <- gRPC服务 <- msg-server消费者
/// ```
/// 
/// ## 服务特性
/// 
/// - **多平台支持**: 同一用户可在多个平台同时在线
/// - **实时通信**: 基于WebSocket的双向通信
/// - **负载均衡**: 支持多实例部署和服务发现
/// - **安全认证**: JWT令牌验证和连接授权
/// - **连接管理**: 自动心跳检测和异常处理
/// - **消息路由**: 智能消息分发和推送策略

mod client;
mod manager;
pub mod rpc;
pub mod ws_server;
