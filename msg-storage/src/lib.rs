use seq::SeqRepo;
use tracing::info;

use common::{config::AppConfig, proto::message::MsgType, error::Error};

mod mongodb;
mod postgres;

pub mod message;
// pub mod rpc;
pub mod seq;

use std::sync::Arc;
use ::sqlx::PgPool;
use message::{MsgRecBoxCleaner, MsgRecBoxRepo, MsgStoreRepo};

/// 数据库仓库结构体，用于管理消息存储和序列号相关的数据库操作
/// 包含消息存储仓库和序列号仓库的实例
#[derive(Debug)]
pub struct DbRepo {
    pub msg: Box<dyn MsgStoreRepo>,
    pub seq: Box<dyn SeqRepo>,
}

impl DbRepo {
    /// 创建新的数据库仓库实例
    /// 根据配置初始化PostgreSQL连接池，并创建消息存储和序列号仓库
    pub async fn new(config: &AppConfig) -> Self {
        let pool = PgPool::connect(&config.database.pg_url()).await.unwrap();
        let seq_step = config.redis.seq_step;

        let msg = Box::new(postgres::PostgresMessage::new(pool.clone()));
        let seq = Box::new(postgres::PostgresSeq::new(pool, seq_step));
        Self {
            msg,
            seq,
        }
    }
}

/// 创建消息接收箱仓库实例
/// 用于管理用户的消息接收箱，基于MongoDB实现
pub async fn msg_rec_box_repo(config: &AppConfig) -> Result<Arc<dyn MsgRecBoxRepo>, Error> {
    let msg_box = mongodb::MsgBox::from_config(config).await?;
    Ok(Arc::new(msg_box))
}

/// 创建消息接收箱清理器实例
/// 用于定期清理过期的消息
pub async fn msg_rec_box_cleaner(config: &AppConfig) -> Result<Arc<dyn MsgRecBoxCleaner>, Error> {
    let msg_box = mongodb::MsgBox::from_config(config).await?;
    Ok(Arc::new(msg_box))
}

/// 清理消息接收箱
/// 启动定期清理任务，删除过期的消息（除了指定类型的消息）
pub async fn clean_receive_box(config: &AppConfig) -> Result<(), Error> {
    let types: Vec<i32> = config
        .database
        .mongodb
        .clean
        .except_types
        .iter()
        .filter_map(|v| MsgType::from_str_name(v))
        .map(|v| v as i32)
        .collect();
    let period = config.database.mongodb.clean.period;

    let msg_box = msg_rec_box_cleaner(config).await?;
    info!(
        "消息接收箱清理任务已启动，清理周期为 {period} 秒；不清理的消息类型为：{:?}",
        types
    );
    msg_box.clean_receive_box(period, types);
    Ok(())
}
