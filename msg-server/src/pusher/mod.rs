use std::{fmt::Debug, sync::Arc};

use common::{
    config::AppConfig,
    error::Error,
    message::{GroupMemSeq, Msg},
};
use tonic::async_trait;

mod service;

#[async_trait]
pub trait Pusher: Send + Sync + Debug {
    async fn push_single_msg(&self, msg: Msg) -> Result<(), Error>;
    async fn push_group_msg(&self, msg: Msg, members: Vec<GroupMemSeq>) -> Result<(), Error>;
}

pub async fn push_service(config: &AppConfig) -> Arc<dyn Pusher> {
    Arc::new(service::PusherService::new(config).await)
}
