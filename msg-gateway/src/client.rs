use common::message::PlatformType;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::stream::SplitSink;
use futures::SinkExt;
use std::sync::Arc;
use axum::body::Bytes;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;

type ClientSender = Arc<RwLock<SplitSink<WebSocket, Message>>>;

/// client
#[derive(Debug)]
pub struct Client {
    // hold a ws connection sender
    pub sender: ClientSender,
    // user id
    pub user_id: String,
    // platform id
    pub platform_id: String,
    pub platform: PlatformType,
    pub notify_sender: Sender<()>,
}

#[allow(dead_code)]
impl Client {
    pub async fn send_text(&self, msg: String) -> Result<(), axum::Error> {
        self.sender.write().await.send(Message::Text(Utf8Bytes::from(msg))).await
    }

    pub async fn send_binary(&self, msg: Vec<u8>) -> Result<(), axum::Error> {
        self.sender.write().await.send(Message::Binary(Bytes::from(msg))).await
    }
}
