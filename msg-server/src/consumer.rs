use std::sync::Arc;
use std::time::Duration;

use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::BorrowedMessage;
use rdkafka::producer::FutureProducer;
use rdkafka::{ClientConfig, Message, Offset};
use tracing::{debug, error, info, warn};

use cache::Cache;
use runtime::config::AppConfig;
use contract::error::Error;
use contract::proto::message::{GroupMemSeq, Msg, MsgRead, MsgType};
use contract::types::msg::MsgType2;
use msg_storage::message::MsgRecBoxRepo;
use msg_storage::{msg_rec_box_repo, DbRepo};

use crate::kafka;
use crate::pusher::{push_service, Pusher};

// 添加群组服务客户端相关导入
use runtime::grpc_client::base::get_rpc_client;
use contract::proto::group::group_service_client::GroupServiceClient;
use contract::proto::group::GetMembersRequest;
use runtime::service_discovery::LbWithServiceDiscovery;

/// 单条消息的最大处理尝试次数（含首次）
const MAX_ATTEMPTS: u32 = 3;

/// 重试的基础退避时间，第 n 次重试等待 n 倍
const RETRY_BACKOFF: Duration = Duration::from_millis(200);

/// 死信主题后缀
const DEAD_LETTER_SUFFIX: &str = ".dlq";

/// 消息消费者服务
///
/// 这是msg-server的核心组件之一，负责：
/// 1. 从Kafka消息队列消费消息
/// 2. 解析和验证消息格式
/// 3. 分配消息序列号
/// 4. 将消息存储到数据库（PostgreSQL + MongoDB）
/// 5. 推送消息到在线用户
/// 6. 处理群聊消息的成员分发
/// 7. 处理已读消息状态更新
///
/// ## 投递语义
///
/// 消费者关闭了自动提交，**只有业务处理成功之后才提交偏移量**，
/// 因此提供的是 at-least-once 而不是 at-most-once：
/// - 处理成功 → 同步提交位点；
/// - 处理失败 → 有限次重试，仍失败则转入死信主题后再提交，避免阻塞分区；
/// - 死信也投递不出去 → 不提交，让 Kafka 重新投递。
///
/// 代价是可能重复处理。重复处理由 `already_processed` 闸门按 `server_id` 拦掉
/// 已经落库的消息，未落库的消息（通话信令等）本身不参与持久化，重复执行的副作用仅为一次推送。
pub struct ConsumerService {
    /// Kafka流式消费者实例
    consumer: StreamConsumer,

    /// 数据库操作仓库
    db: Arc<DbRepo>,

    /// MongoDB消息盒子仓库
    msg_box: Arc<dyn MsgRecBoxRepo>,

    /// 消息推送器
    pusher: Arc<dyn Pusher>,

    /// 缓存接口
    cache: Arc<dyn Cache>,

    /// 群组服务客户端
    group_client: GroupServiceClient<LbWithServiceDiscovery>,

    /// 死信生产者
    dlq_producer: FutureProducer,

    /// 死信主题名
    dead_letter_topic: String,

    /// 主主题名，仅用于日志
    topic: String,
}

impl ConsumerService {
    /// 创建新的消息消费者服务实例
    pub async fn new(config: &AppConfig) -> Result<Self, Error> {
        // 关闭自动提交：位点必须由业务处理结果决定
        let consumer: StreamConsumer = ClientConfig::new()
            .set("group.id", &config.kafka.group)
            .set("bootstrap.servers", config.kafka.hosts.join(","))
            .set("enable.partition.eof", "false")
            .set("session.timeout.ms", config.kafka.consumer.session_timeout.to_string())
            .set("enable.auto.commit", "false")
            .set("auto.offset.reset", &config.kafka.consumer.auto_offset_reset)
            .create()
            .map_err(|e| Error::Internal(format!("Kafka消费者创建失败: {}", e)))?;

        consumer
            .subscribe(&[&config.kafka.topic])
            .map_err(|e| Error::Internal(format!("无法订阅Kafka主题 '{}': {}", config.kafka.topic, e)))?;

        info!("Kafka消费者已初始化（手动提交），订阅主题: {}", config.kafka.topic);

        // 死信主题与生产者：处理失败的消息最终落到这里，而不是被静默丢弃
        let dead_letter_topic = format!("{}{}", config.kafka.topic, DEAD_LETTER_SUFFIX);
        let dlq_producer = kafka::build_producer(config)?;
        kafka::ensure_topic(
            &dead_letter_topic,
            &config.kafka.hosts.join(","),
            config.kafka.connect_timeout as u16,
        )
        .await
        .map_err(|e| Error::Internal(format!("确保死信主题存在失败: {}", e)))?;

        info!("死信主题已就绪: {}", dead_letter_topic);

        let pusher = push_service(config).await?;
        let db = Arc::new(DbRepo::new(config).await);
        let cache = cache::cache(config).await;
        let msg_box = msg_rec_box_repo(config).await?;
        let group_client =
            get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(config, "group".to_string())
                .await?;

        info!("消息消费者服务初始化完成");

        Ok(Self {
            consumer,
            db,
            msg_box,
            pusher,
            cache,
            group_client,
            dlq_producer,
            dead_letter_topic,
            topic: config.kafka.topic.clone(),
        })
    }

    /// 启动消息消费循环
    ///
    /// 会一直运行直到服务停止或发生致命错误。
    pub async fn consume(&mut self) -> Result<(), Error> {
        info!(
            "开始消费Kafka消息: topic={}, 手动提交, 单条最多尝试 {} 次, 死信主题={}",
            self.topic, MAX_ATTEMPTS, self.dead_letter_topic
        );

        loop {
            let message = match self.consumer.recv().await {
                Ok(message) => message,
                Err(e) => {
                    error!("从Kafka接收消息失败: {}", e);
                    // 接收错误通常是瞬时故障，退避后重试而不是退出服务
                    tokio::time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
            };

            // 先把负载取成拥有所有权的字符串，后续处理不再借用 message
            let payload = match message.payload_view::<str>() {
                Some(Ok(payload)) => payload.to_owned(),
                Some(Err(e)) => {
                    error!("消息负载不是合法的 UTF-8，无法重放，直接跳过: {}", e);
                    Self::commit(&self.consumer, &message, "负载解码失败");
                    continue;
                }
                None => {
                    warn!("收到空消息，直接跳过");
                    Self::commit(&self.consumer, &message, "空消息");
                    continue;
                }
            };

            match retry_with_backoff(MAX_ATTEMPTS, RETRY_BACKOFF, || self.handle_msg(&payload))
                .await
            {
                Ok(()) => {
                    debug!("消息处理成功，提交偏移量，长度: {} 字节", payload.len());
                    Self::commit(&self.consumer, &message, "处理成功");
                }
                Err(e) => {
                    error!("消息处理失败且重试耗尽: {:?}", e);
                    self.handle_failure(&message, &payload, &e.to_string()).await;
                }
            }
        }
    }

    /// 处理失败后的收尾：优先转入死信，失败则保留位点等待重投
    async fn handle_failure(
        &self,
        message: &BorrowedMessage<'_>,
        payload: &str,
        reason: &str,
    ) {
        match kafka::send_to_topic(&self.dlq_producer, &self.dead_letter_topic, payload).await {
            Ok(()) => {
                error!(
                    "消息处理失败({})，已转入死信主题 {}",
                    reason, self.dead_letter_topic
                );
                Self::commit(&self.consumer, message, "已转入死信主题");
            }
            Err(e) => {
                // 死信都写不进去说明下游整体不可用，此时提交位点等于丢消息。
                // 必须把消费位置退回当前消息，否则下一条 recv 会直接越过它。
                warn!(
                    "死信投递失败({})，回退偏移量等待重新投递",
                    e
                );
                Self::rewind(&self.consumer, message);
                tokio::time::sleep(RETRY_BACKOFF).await;
            }
        }
    }

    /// 把消费位置退回指定消息，让下一次 `recv` 重新拿到它
    fn rewind(consumer: &StreamConsumer, message: &BorrowedMessage<'_>) {
        let offset = message.offset();
        if let Err(e) = consumer.seek(
            message.topic(),
            message.partition(),
            Offset::Offset(offset),
            Duration::from_secs(1),
        ) {
            // 退不回去就只能记下来人工补偿
            error!("回退偏移量 {} 失败，该消息可能丢失: {:?}", offset, e);
        }
    }

    /// 同步提交偏移量
    fn commit(consumer: &StreamConsumer, message: &BorrowedMessage<'_>, reason: &str) {
        if let Err(e) = consumer.commit_message(message, CommitMode::Sync) {
            // 提交失败意味着重启后会重复消费，由幂等闸门兜底
            error!("提交偏移量失败({}): {:?}", reason, e);
        }
    }

    /// 处理单条消息的核心业务逻辑
    ///
    /// 这里只做编排，具体分支交给下面几个私有方法。
    async fn handle_msg(&self, payload: &str) -> Result<(), Error> {
        debug!("开始处理消息: {}", payload);

        let mut msg: Msg = serde_json::from_str(payload)
            .map_err(|e| Error::Internal(format!("消息JSON解析失败: {}", e)))?;

        // 幂等闸门：位点回退、重复投递都会让同一条消息再次进来
        if already_processed(&*self.msg_box, &msg.server_id).await? {
            info!(
                "消息已处理过，跳过重复投递: server_id={}",
                msg.server_id
            );
            return Ok(());
        }

        let mt = MsgType::try_from(msg.msg_type)
            .map_err(|e| Error::Internal(format!("未知的消息类型 {}: {}", msg.msg_type, e)))?;

        // 已读消息有独立处理逻辑，不参与序列号分配与持久化
        if mt == MsgType::Read {
            debug!("处理已读消息: user_id={}", msg.receiver_id);
            return self.handle_msg_read(msg).await;
        }

        let (msg_type, need_increase_seq, need_history) = MsgType2::classify_msg_type(mt);
        debug!(
            "消息分类结果: {:?}, 需要序列号: {}, 需要历史: {}",
            msg_type, need_increase_seq, need_history
        );

        // 接收序列号
        if need_increase_seq {
            let cur_seq = self.increase_message_seq(&msg.receiver_id).await?;
            msg.seq = cur_seq;
            debug!("为用户 {} 分配序列号: {}", msg.receiver_id, cur_seq);
        }

        // 群聊消息需要按成员维度各自分配序列号
        let members = self.handle_group_seq(&msg_type, &mut msg).await?;

        let mut tasks = Vec::with_capacity(2);

        if Self::get_send_to_db_flag(&mt) && !msg.is_revoked {
            let cloned_msg = msg.clone();
            let cloned_type = msg_type.clone();
            let cloned_members = members.clone();
            let db = self.db.clone();
            let msg_box = self.msg_box.clone();

            let to_db = tokio::spawn(async move {
                debug!("开始存储消息到数据库: server_id={}", cloned_msg.server_id);
                Self::send_to_db(
                    db,
                    msg_box,
                    cloned_msg,
                    cloned_type,
                    need_history,
                    cloned_members,
                )
                .await
            });

            tasks.push(to_db);
        }

        let pusher = self.pusher.clone();
        let to_pusher = tokio::spawn(async move {
            debug!(
                "开始推送消息: server_id={}, 类型={:?}",
                msg.server_id, msg_type
            );
            match msg_type {
                MsgType2::Friend => pusher.push_single_msg(msg).await,
                MsgType2::Group => pusher.push_group_msg(msg, members).await,
                MsgType2::System => {
                    // TODO系统消息暂未实现
                    warn!("系统消息暂未实现: {:?}", msg_type);
                    Ok(())
                }
            }
        });

        tasks.push(to_pusher);

        // 落库与推送必须都成功才算处理完成，否则位点不推进
        for task in tasks {
            task.await
                .map_err(|e| Error::Internal(format!("消息处理任务执行失败: {}", e)))??;
        }

        Ok(())
    }

    /// 从缓存查询群组成员ID列表，缓存缺失时回源群组服务
    async fn get_members_id(&self, group_id: &str) -> Result<Vec<String>, Error> {
        match self.cache.query_group_members_id(group_id).await {
            Ok(list) if !list.is_empty() => Ok(list),
            Ok(_) => {
                warn!("缓存中群组成员ID列表为空，回源群组服务");
                self.query_group_members_id_from_db(group_id).await
            }
            Err(err) => {
                error!("从缓存查询群组成员ID失败: {:?}", err);
                Err(err)
            }
        }
    }

    /// 为用户增加消息接收序列号
    async fn increase_message_seq(&self, user_id: &str) -> Result<i64, Error> {
        let (cur_seq, _, updated) = self.cache.increase_seq(user_id).await?;
        if updated {
            self.db.seq.save_max_seq(user_id).await?;
        }
        Ok(cur_seq)
    }

    /// 处理消息已读状态更新
    async fn handle_msg_read(&self, msg: Msg) -> Result<(), Error> {
        let data: MsgRead = bincode::deserialize(&msg.content)
            .map_err(|_| Error::Internal("反序列化MsgRead失败".to_string()))?;

        self.msg_box.msg_read(&data.user_id, &data.msg_seq).await?;
        Ok(())
    }

    /// 处理群聊消息的序列号分配
    ///
    /// 群解散、成员退出等操作会顺带清理缓存中的群成员信息。
    async fn handle_group_seq(
        &self,
        msg_type: &MsgType2,
        msg: &mut Msg,
    ) -> Result<Vec<GroupMemSeq>, Error> {
        if *msg_type != MsgType2::Group {
            return Ok(vec![]);
        }

        let mut members = self.get_members_id(&msg.receiver_id).await?;

        // 排除发送者自己（发送者不需要接收自己的消息）
        members.retain(|id| id != &msg.send_id);

        self.cache.incr_group_seq(members).await
    }

    /// 判断消息是否需要存储到数据库
    ///
    /// 音视频通话信令这类消息只有实时价值，落地存储反而是噪音。
    #[inline]
    fn get_send_to_db_flag(msg_type: &MsgType) -> bool {
        !matches!(
            *msg_type,
            MsgType::ConnectSingleCall
                | MsgType::AgreeSingleCall
                | MsgType::Candidate
                | MsgType::SingleCallOffer
                | MsgType::SingleCallInvite
                | MsgType::FriendApplyReq
        )
    }

    /// 将消息存储到数据库
    async fn send_to_db(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        msg: Msg,
        msg_type: MsgType2,
        need_to_history: bool,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        match msg_type {
            MsgType2::Friend => {
                Self::handle_message(db, msg_box, msg, need_to_history).await?;
            }
            MsgType2::Group => {
                Self::handle_group_message(db, msg_box, msg, need_to_history, members).await?;
            }
            MsgType2::System => {
                // TODO系统消息暂未实现
                warn!("系统消息暂未实现")
            }
        }

        Ok(())
    }

    /// 从群组服务查询群组成员ID并回填缓存
    async fn query_group_members_id_from_db(&self, group_id: &str) -> Result<Vec<String>, Error> {
        info!("从群组服务查询群组成员ID: {}", group_id);

        // tonic 客户端 clone 只是复制 channel 引用，不产生新连接
        let mut group_client = self.group_client.clone();
        let request = GetMembersRequest {
            group_id: group_id.to_string(),
            page: 1,
            page_size: 5000,
        };

        match group_client.get_members(request).await {
            Ok(response) => {
                let members_id: Vec<String> = response
                    .into_inner()
                    .members
                    .into_iter()
                    .map(|member| member.user_id)
                    .collect();

                info!("从群组服务查询到 {} 个成员ID", members_id.len());

                if let Err(e) = self
                    .cache
                    .save_group_members_id(group_id, members_id.clone())
                    .await
                {
                    error!("保存群组成员ID到缓存失败: {:?}", e);
                }

                Ok(members_id)
            }
            Err(e) => {
                // 这里返回错误而不是空列表：返回空列表会让消息静默地不落任何人的盒子
                error!("从群组服务查询成员ID失败: {:?}", e);
                Err(Error::Internal(format!("查询群组成员失败: {}", e)))
            }
        }
    }

    /// 处理单聊消息的存储
    async fn handle_message(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        message: Msg,
        need_to_history: bool,
    ) -> Result<(), Error> {
        let mut tasks = Vec::with_capacity(2);

        if need_to_history {
            let cloned_msg = message.clone();
            let db_task = tokio::spawn(async move { db.msg.save_message(cloned_msg).await });
            tasks.push(db_task);
        }

        let msg_rec_box_task = tokio::spawn(async move {
            // 群组操作确认 / 邀请确认类消息：从消息盒子删除而不是新增
            if message.msg_type == MsgType::GroupDismissOrExitReceived as i32
                || message.msg_type == MsgType::GroupInvitationReceived as i32
            {
                return msg_box.delete_message(&message.server_id).await;
            }
            msg_box.save_message(&message).await
        });
        tasks.push(msg_rec_box_task);

        for task in tasks {
            task.await
                .map_err(|e| Error::Internal(e.to_string()))??;
        }
        Ok(())
    }

    /// 处理群聊消息的存储
    async fn handle_group_message(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        message: Msg,
        need_to_history: bool,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        // 需要把序列号水位推进到数据库的成员
        let need_update = members
            .iter()
            .filter(|item| item.need_update)
            .map(|item| item.mem_id.clone())
            .collect::<Vec<String>>();

        let cloned_msg = if need_to_history {
            Some(message.clone())
        } else {
            None
        };

        let db_task = tokio::spawn(async move {
            if !need_update.is_empty() {
                db.seq.save_max_seq_batch(&need_update).await?;
            }
            if let Some(cloned_msg) = cloned_msg {
                db.msg.save_message(cloned_msg).await?;
            }
            Ok::<(), Error>(())
        });

        // 群解散消息不进入成员的消息盒子
        let should_save_to_mongodb = message.msg_type != MsgType::GroupDismiss as i32;
        let msg_rec_box_task = tokio::spawn(async move {
            if should_save_to_mongodb {
                msg_box.save_group_msg(message, members).await?;
            } else {
                info!("群解散消息不保存到MongoDB: server_id={}", message.server_id);
            }
            Ok::<(), Error>(())
        });

        let (db_result, msg_rec_box_result) = tokio::try_join!(db_task, msg_rec_box_task)
            .map_err(|e| Error::Internal(e.to_string()))?;

        db_result?;
        msg_rec_box_result?;

        Ok(())
    }
}

/// 幂等闸门：判断一条消息是否已经落过库
///
/// 以 `server_id` 为准 —— 它在生产端由 `nanoid` 生成，是全局唯一且稳定的。
pub(crate) async fn already_processed(
    repo: &dyn MsgRecBoxRepo,
    server_id: &str,
) -> Result<bool, Error> {
    if server_id.is_empty() {
        // 历史数据可能存在空 ID，无法据此判定重复，放行交给后续流程
        return Ok(false);
    }
    Ok(repo.get_message(server_id).await?.is_some())
}

/// 带退避的有限重试
///
/// 重试的是「整条消息的处理」，因此可能重复产生副作用；
/// 下游用 `server_id` 去重。相比直接丢消息，重复是更可接受的失败模式。
async fn retry_with_backoff<F, Fut>(
    max_attempts: u32,
    base_backoff: Duration,
    mut op: F,
) -> Result<(), Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    let attempts = max_attempts.max(1);
    let mut last_err: Option<Error> = None;

    for attempt in 1..=attempts {
        match op().await {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!("第 {}/{} 次处理消息失败: {}", attempt, attempts, e);
                last_err = Some(e);

                if let Some(backoff) = retry_backoff(attempt, attempts, base_backoff) {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| Error::Internal("未执行任何处理尝试".to_string())))
}

/// 计算下一次重试前的等待时长，返回 `None` 表示重试次数已用尽
fn retry_backoff(attempt: u32, max_attempts: u32, base: Duration) -> Option<Duration> {
    if attempt >= max_attempts {
        None
    } else {
        Some(base * attempt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contract::proto::message::Msg;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::sync::mpsc;

    /// 只实现幂等闸门用得到的 `get_message`，其余方法不应被调用
    #[derive(Debug)]
    struct FakeMsgBox {
        existing: Option<Msg>,
        fail: bool,
    }

    #[async_trait]
    impl MsgRecBoxRepo for FakeMsgBox {
        async fn save_message(&self, _message: &Msg) -> Result<(), Error> {
            unimplemented!()
        }

        async fn save_group_msg(
            &self,
            _message: Msg,
            _members: Vec<GroupMemSeq>,
        ) -> Result<(), Error> {
            unimplemented!()
        }

        async fn delete_message(&self, _message_id: &str) -> Result<(), Error> {
            unimplemented!()
        }

        async fn delete_messages(&self, _user_id: &str, _msg_seq: Vec<i64>) -> Result<(), Error> {
            unimplemented!()
        }

        async fn delete_group_messages(&self, _group_id: &str) -> Result<i64, Error> {
            unimplemented!()
        }

        async fn revoke_message(&self, _message_id: &str, _user_id: &str) -> Result<(), Error> {
            unimplemented!()
        }

        async fn get_message(&self, _message_id: &str) -> Result<Option<Msg>, Error> {
            if self.fail {
                return Err(Error::Internal("mongo 不可用".to_string()));
            }
            Ok(self.existing.clone())
        }

        async fn get_messages_stream(
            &self,
            _user_id: &str,
            _start: i64,
            _end: i64,
        ) -> Result<mpsc::Receiver<Result<Msg, Error>>, Error> {
            unimplemented!()
        }

        async fn get_messages(&self, _message_ids: &Vec<String>) -> Result<Vec<Msg>, Error> {
            unimplemented!()
        }

        async fn get_msgs(
            &self,
            _user_id: &str,
            _send_start: i64,
            _send_end: i64,
            _rec_start: i64,
            _rec_end: i64,
        ) -> Result<Vec<Msg>, Error> {
            unimplemented!()
        }

        async fn get_conversation_messages_by_seq_range(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _send_seq_start: i64,
            _send_seq_end: i64,
            _seq_start: i64,
            _seq_end: i64,
        ) -> Result<Vec<Msg>, Error> {
            unimplemented!()
        }

        async fn msg_read(&self, _user_id: &str, _msg_seq: &[i64]) -> Result<(), Error> {
            unimplemented!()
        }

        async fn mark_conversation_read(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _up_to_time: Option<i64>,
        ) -> Result<i32, Error> {
            unimplemented!()
        }

        async fn delete_messages_by_ids(
            &self,
            _user_id: &str,
            _message_ids: &[String],
        ) -> Result<i32, Error> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn duplicate_message_is_rejected_by_gate() {
        let repo = FakeMsgBox {
            existing: Some(Msg {
                server_id: "s1".into(),
                ..Default::default()
            }),
            fail: false,
        };

        assert!(
            already_processed(&repo, "s1").await.unwrap(),
            "已落库的消息必须被判定为重复，避免位点回退后重复计数"
        );
    }

    #[tokio::test]
    async fn fresh_message_passes_gate() {
        let repo = FakeMsgBox {
            existing: None,
            fail: false,
        };

        assert!(!already_processed(&repo, "s1").await.unwrap());
    }

    #[tokio::test]
    async fn empty_server_id_is_never_treated_as_duplicate() {
        let repo = FakeMsgBox {
            existing: Some(Msg::default()),
            fail: false,
        };

        // 空 ID 无法判定，放行，否则历史脏数据会被永久卡住
        assert!(!already_processed(&repo, "").await.unwrap());
    }

    #[tokio::test]
    async fn gate_propagates_storage_error() {
        let repo = FakeMsgBox {
            existing: None,
            fail: true,
        };

        // 判定不出来就不能当作「没处理过」，否则等于放弃了幂等保护
        assert!(already_processed(&repo, "s1").await.is_err());
    }

    #[tokio::test]
    async fn retry_stops_after_max_attempts() {
        let calls = AtomicU32::new(0);

        let result = retry_with_backoff(3, Duration::ZERO, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(Error::Internal("boom".to_string()))
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3, "重试次数必须收敛，不能无限重试");
    }

    #[tokio::test]
    async fn retry_stops_at_first_success() {
        let calls = AtomicU32::new(0);

        let result = retry_with_backoff(5, Duration::ZERO, || async {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 2 {
                Err(Error::Internal("transient".to_string()))
            } else {
                Ok(())
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn backoff_grows_and_stops() {
        let base = Duration::from_millis(100);

        assert_eq!(retry_backoff(1, 3, base), Some(Duration::from_millis(100)));
        assert_eq!(retry_backoff(2, 3, base), Some(Duration::from_millis(200)));
        assert_eq!(retry_backoff(3, 3, base), None, "最后一次尝试后不应再等待");
        assert_eq!(retry_backoff(4, 3, base), None);
    }
}
