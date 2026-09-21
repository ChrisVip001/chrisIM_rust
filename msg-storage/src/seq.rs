use std::fmt::Debug;

use tokio::sync::mpsc::Receiver;

use common::error::Error;
use tonic::async_trait;

/// 序列号仓库trait
/// 用于管理用户的消息序列号，包括发送序列号和接收序列号
#[async_trait]
pub trait SeqRepo: Sync + Send + Debug {
    /// 保存并递增用户的发送最大序列号
    /// 返回更新后的发送最大序列号
    async fn save_send_max_seq(&self, user_id: &str) -> Result<i64, Error>;
    
    /// 保存并递增用户的接收最大序列号
    /// 返回更新后的接收最大序列号
    async fn save_max_seq(&self, user_id: &str) -> Result<i64, Error>;
    
    /// 批量更新多个用户的接收最大序列号
    /// 用于群组消息等需要同时更新多个用户序列号的场景
    async fn save_max_seq_batch(&self, user_ids: &[String]) -> Result<(), Error>;
    
    /// 获取所有用户的序列号信息
    /// 返回包含用户ID、发送最大序列号、接收最大序列号的流
    async fn get_max_seq(&self) -> Result<Receiver<(String, i64, i64)>, Error>;
}
