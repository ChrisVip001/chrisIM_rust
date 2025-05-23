use seq::SeqRepo;
use tracing::info;

use common::{config::AppConfig, message::MsgType, error::Error};

mod mongodb;
mod postgres;

pub mod message;
// pub mod rpc;
pub mod seq;

use std::sync::Arc;
use ::sqlx::PgPool;
use message::{MsgRecBoxCleaner, MsgRecBoxRepo, MsgStoreRepo};

/// shall we create a structure to hold everything we need?
/// like db pool and mongodb's database
#[derive(Debug)]
pub struct DbRepo {
    pub msg: Box<dyn MsgStoreRepo>,
    pub seq: Box<dyn SeqRepo>,
}

impl DbRepo {
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

pub async fn msg_rec_box_repo(config: &AppConfig) -> Result<Arc<dyn MsgRecBoxRepo>, Error> {
    let msg_box = mongodb::MsgBox::from_config(config).await?;
    Ok(Arc::new(msg_box))
}

pub async fn msg_rec_box_cleaner(config: &AppConfig) -> Result<Arc<dyn MsgRecBoxCleaner>, Error> {
    let msg_box = mongodb::MsgBox::from_config(config).await?;
    Ok(Arc::new(msg_box))
}

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
        "clean receive box task started, and the period is {period}s; the except types is {:?}",
        types
    );
    msg_box.clean_receive_box(period, types);
    Ok(())
}
