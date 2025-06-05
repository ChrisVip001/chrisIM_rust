use std::fmt::Debug;

use async_trait::async_trait;
use tokio::sync::mpsc;

use common::error::Error;
use common::proto::message::{GroupMemSeq, Msg};

/// 消息存储仓库trait
/// 面向PostgreSQL数据库，用于持久化存储消息
#[async_trait]
pub trait MsgStoreRepo: Sync + Send + Debug {
    /// 保存消息到数据库
    async fn save_message(&self, message: Msg) -> Result<(), Error>;
}

/// 消息接收箱仓库trait
/// 面向MongoDB数据库
/// 当用户接收消息时，会从接收箱中删除消息
#[async_trait]
pub trait MsgRecBoxRepo: Sync + Send + Debug {
    /// 保存消息，需要消息结构体
    async fn save_message(&self, message: &Msg) -> Result<(), Error>;

    /// 保存消息到消息接收箱
    /// 需要群组成员ID列表
    async fn save_group_msg(&self, message: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error>;

    /// 根据消息ID删除单条消息
    async fn delete_message(&self, message_id: &str) -> Result<(), Error>;

    /// 根据用户ID和消息序列号批量删除消息
    async fn delete_messages(&self, user_id: &str, msg_seq: Vec<i64>) -> Result<(), Error>;

    #[allow(dead_code)]
    /// 根据消息ID获取单条消息
    async fn get_message(&self, message_id: &str) -> Result<Option<Msg>, Error>;

    /// 从接收箱获取消息流
    /// 使用流式处理，支持大量消息的高效传输
    async fn get_messages_stream(
        &self,
        user_id: &str,
        start: i64,
        end: i64,
    ) -> Result<mpsc::Receiver<Result<Msg, Error>>, Error>;

    #[deprecated]
    /// 获取消息列表（已废弃，建议使用流式接口）
    async fn get_messages(&self, user_id: &str, start: i64, end: i64) -> Result<Vec<Msg>, Error>;

    /// 获取用户的发送和接收消息
    /// 支持分别指定发送消息和接收消息的序列号范围
    async fn get_msgs(
        &self,
        user_id: &str,
        send_start: i64,
        send_end: i64,
        rec_start: i64,
        rec_end: i64,
    ) -> Result<Vec<Msg>, Error>;

    /// 根据用户ID和消息序列号更新消息已读状态
    async fn msg_read(&self, user_id: &str, msg_seq: &[i64]) -> Result<(), Error>;
}

/// 消息接收箱清理器trait
/// 用于定期清理过期消息
pub trait MsgRecBoxCleaner: Sync + Send {
    /// 运行消息接收箱清理任务
    /// 清理所有消息，除了群组操作相关的消息类型
    ///
    /// # 参数
    /// * period: 清理周期，单位为天
    /// * types: 不清理的消息类型列表，如群组操作相关消息；使用MsgType枚举值
    ///
    fn clean_receive_box(&self, period: i64, types: Vec<i32>);
}
