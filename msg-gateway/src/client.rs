use axum::body::Bytes;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use common::proto::message::PlatformType;
use futures::stream::SplitSink;
use futures::SinkExt;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;

/// WebSocket客户端发送器类型别名
/// 
/// 使用Arc<RwLock<>>包装以支持多线程安全访问
/// SplitSink是WebSocket连接的发送端，用于向客户端发送消息
type ClientSender = Arc<RwLock<SplitSink<WebSocket, Message>>>;

/// WebSocket客户端连接封装
/// 
/// 代表一个已连接的客户端，包含了客户端的所有必要信息：
/// - WebSocket连接的发送端
/// - 用户身份标识
/// - 平台信息
/// - 通知机制
/// 
/// 每个客户端对应一个特定用户在特定平台上的连接
#[derive(Debug)]
pub struct Client {
    /// WebSocket连接发送器
    /// 用于向客户端发送消息，支持并发访问
    pub sender: ClientSender,
    
    /// 用户ID
    /// 唯一标识用户身份，用于消息路由
    pub user_id: String,
    
    /// 平台ID
    /// 客户端的唯一标识符，用于区分同一用户的不同连接
    pub platform_id: String,
    
    /// 平台类型
    /// 标识客户端运行的平台（桌面端或移动端）
    pub platform: PlatformType,
    
    /// 通知发送器
    /// 用于发送踢下线等控制信号
    pub notify_sender: Sender<()>,
}

#[allow(dead_code)]
impl Client {
    /// 向客户端发送文本消息
    /// 
    /// 将字符串消息通过WebSocket连接发送给客户端。
    /// 消息会被编码为UTF-8格式的文本帧。
    /// 
    /// # 参数
    /// * `msg` - 要发送的文本消息
    /// 
    /// # 返回值
    /// * `Ok(())` - 发送成功
    /// * `Err(axum::Error)` - 发送失败，通常是连接已断开
    /// 
    /// # 使用场景
    /// - 发送JSON格式的控制消息
    /// - 发送简单的文本通知
    /// - 调试和测试用途
    pub async fn send_text(&self, msg: String) -> Result<(), axum::Error> {
        self.sender
            .write()
            .await
            .send(Message::Text(Utf8Bytes::from(msg)))
            .await
    }

    /// 向客户端发送二进制消息
    /// 
    /// 将字节数据通过WebSocket连接发送给客户端。
    /// 这是发送序列化消息对象的主要方式。
    /// 
    /// # 参数
    /// * `msg` - 要发送的二进制数据
    /// 
    /// # 返回值
    /// * `Ok(())` - 发送成功
    /// * `Err(axum::Error)` - 发送失败，通常是连接已断开
    /// 
    /// # 使用场景
    /// - 发送bincode序列化的消息对象
    /// - 发送图片、文件等二进制内容
    /// - 高效的数据传输
    pub async fn send_binary(&self, msg: Vec<u8>) -> Result<(), axum::Error> {
        self.sender
            .write()
            .await
            .send(Message::Binary(Bytes::from(msg)))
            .await
    }
}
