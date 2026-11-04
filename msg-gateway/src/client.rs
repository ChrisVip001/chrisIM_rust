use axum::body::Bytes;
use axum::extract::ws::{Message, Utf8Bytes};
use contract::proto::message::PlatformType;
use tokio::sync::mpsc::Sender;

/// 下行发送失败类型
///
/// 仅在队列为空且已关闭（写任务退出、连接销毁）时出现，
/// 语义等价于「连接已不可用」。
pub type SendError = tokio::sync::mpsc::error::SendError<Message>;

/// 每连接下行发送队列容量
///
/// 有界队列天然形成背压：客户端消费过慢时，推送方在发送处等待，
/// 而不是在某个全局锁上排队。128 条足够吸收瞬时推送洪峰，
/// 同时把最坏情况下的每连接内存占用限制在可预算的范围内。
pub(crate) const OUTBOUND_CHANNEL_CAP: usize = 128;

/// 客户端连接封装
///
/// 代表一个已连接的客户端，包含了客户端的所有必要信息：
/// - 下行发送队列（由独立的写任务消费，写端唯一，天然无锁竞争）
/// - 用户身份标识
/// - 平台信息
/// - 通知机制
///
/// 每个客户端对应一个特定用户在特定平台上的连接。
#[derive(Debug)]
pub struct Client {
    /// 下行发送队列
    ///
    /// 所有写操作（推送、心跳、关闭帧）统一进入该队列，
    /// 由 [`crate::ws_server`] 中的唯一写任务串行写入 WebSocket，
    /// 彻底消除此前 `RwLock<SplitSink>` 的锁竞争。
    pub sender: Sender<Message>,

    /// 用户ID
    /// 唯一标识用户身份，用于消息路由
    pub user_id: String,

    /// 平台类型
    /// 标识客户端运行的平台（桌面端或移动端）
    pub platform: PlatformType,

    /// 通知发送器
    /// 用于发送踢下线等控制信号
    pub notify_sender: Sender<()>,
}

impl Client {
    /// 入队一条下行消息
    ///
    /// 队列已满时等待（背压）；连接已销毁（写任务退出且队列关闭）时失败。
    pub async fn send_message(&self, msg: Message) -> Result<(), SendError> {
        self.sender.send(msg).await
    }

    /// 向客户端发送文本消息
    ///
    /// 内部会完成 String → 帧数据的转换，适合一次性报文；
    /// 需要多连接复用同一份报文时请用 [`Client::send_frame`]，避免重复拷贝。
    pub async fn send_text(&self, msg: String) -> Result<(), SendError> {
        self.send_frame(Utf8Bytes::from(msg)).await
    }

    /// 发送一份已编码的文本帧
    ///
    /// `Utf8Bytes` 基于 `Bytes`，克隆是引用计数操作（O(1)），
    /// 群推送等「一份数据、多个接收者」的场景应复用同一个实例。
    pub async fn send_frame(&self, frame: Utf8Bytes) -> Result<(), SendError> {
        self.sender.send(Message::Text(frame)).await
    }

    /// 向客户端发送二进制消息
    ///
    /// 这是发送序列化消息对象的主要方式（bincode 等）。
    pub async fn send_binary(&self, msg: Vec<u8>) -> Result<(), SendError> {
        self.sender.send(Message::Binary(Bytes::from(msg))).await
    }
}
