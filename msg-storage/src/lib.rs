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

    // 过期时间为30天
    let period = 30;

    // 保留的消息类型（不会被清理）
    let preserve_types = vec![
        MsgType::FriendApplyReq as i32,
        MsgType::FriendApplyResp as i32,
        MsgType::GroupInvitation as i32,
    ];

    // 启动清理任务
    msg_box.clean_receive_box(period, preserve_types);
    
    info!("消息接收箱清理任务已启动，保留周期: {} 天", period);

    Ok(())
}

/// 初始化序列号缓存
/// 从数据库加载所有用户的序列号信息到Redis缓存中
/// 这是系统启动时的重要步骤，确保序列号的连续性
pub async fn init_seq_cache(config: &AppConfig) -> Result<(), Error> {
    use cache::Cache;
    
    // 创建缓存实例
    let cache = cache::cache(config).await;
    
    // 检查序列号是否已经加载
    if !cache.check_seq_loaded().await? {
        info!("序列号已在缓存中，跳过加载");
        return Ok(());
    }
    
    info!("开始从数据库加载序列号到缓存...");
    
    // 创建数据库仓库
    let db_repo = DbRepo::new(config).await;
    
    // 从数据库获取所有用户的序列号
    let mut seq_receiver = db_repo.seq.get_max_seq().await?;
    
    let mut seq_data = Vec::new();
    let mut count = 0;
    
    // 收集所有序列号数据
    while let Some((user_id, send_max_seq, rec_max_seq)) = seq_receiver.recv().await {
        seq_data.push((user_id, send_max_seq, rec_max_seq));
        count += 1;
        
        // 批量处理，每1000条记录处理一次
        if seq_data.len() >= 1000 {
            cache.set_seq(&seq_data).await?;
            info!("已加载 {} 个用户的序列号到缓存", seq_data.len());
            seq_data.clear();
        }
    }
    
    // 处理剩余的数据
    if !seq_data.is_empty() {
        cache.set_seq(&seq_data).await?;
        info!("已加载剩余 {} 个用户的序列号到缓存", seq_data.len());
    }
    
    // 标记序列号已加载完成
    cache.set_seq_loaded().await?;
    
    info!("序列号缓存初始化完成，共加载 {} 个用户的序列号", count);
    
    Ok(())
}
