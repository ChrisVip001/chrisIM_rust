use std::fmt::Debug;

use async_trait::async_trait;
use tokio::sync::mpsc;

use common::error::Error;
use common::message::{GroupMemSeq, Msg};

/// 面向 postgres 数据库
#[async_trait]
pub trait MsgStoreRepo: Sync + Send + Debug {
    /// 保存消息到数据库
    async fn save_message(&self, message: Msg) -> Result<(), Error>;
}

/// 消息接收箱
/// 面向 mongodb
/// 当用户接收消息时，将从接收箱中删除消息
#[async_trait]
pub trait MsgRecBoxRepo: Sync + Send + Debug {
    /// 保存消息，需要消息结构体
    async fn save_message(&self, message: &Msg) -> Result<(), Error>;

    /// 保存消息到消息接收箱
    /// 需要群组成员ID
    async fn save_group_msg(&self, message: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error>;

    async fn delete_message(&self, message_id: &str) -> Result<(), Error>;

    async fn delete_messages(&self, user_id: &str, msg_seq: Vec<i64>) -> Result<(), Error>;

    #[allow(dead_code)]
    async fn get_message(&self, message_id: &str) -> Result<Option<Msg>, Error>;

    /// 需要考虑如何从接收箱获取消息，
    /// 使用流？还是使用分页？优先选择流
    async fn get_messages_stream(
        &self,
        user_id: &str,
        start: i64,
        end: i64,
    ) -> Result<mpsc::Receiver<Result<Msg, Error>>, Error>;

    #[deprecated]
    async fn get_messages(&self, user_id: &str, start: i64, end: i64) -> Result<Vec<Msg>, Error>;

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

pub trait MsgRecBoxCleaner: Sync + Send {
    /// 运行一个使用 tokio 清理消息接收箱的任务
    /// 清理所有消息，除了群组操作相关类型的消息
    ///
    /// # 参数
    /// * period: 清理的时间周期，单位为天
    /// * types: 不清理的消息类型，如群组操作相关；使用 MsgType
    ///
    fn clean_receive_box(&self, period: i64, types: Vec<i32>);
}
