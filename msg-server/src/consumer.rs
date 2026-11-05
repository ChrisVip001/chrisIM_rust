use std::sync::Arc;
use std::time::{Duration, Instant};

use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::BorrowedMessage;
use rdkafka::producer::FutureProducer;
use rdkafka::{ClientConfig, Message, Offset};
use tracing::{debug, error, info, warn};

use cache::RelationCache;
use runtime::config::AppConfig;
use contract::error::Error;
use contract::proto::message::{GroupMemSeq, Msg, MsgRead, MsgType};
use contract::types::msg::MsgType2;
use contract::types::conversation::conversation_id_of;
use msg_storage::read_spread::MsgMetaRepo;
use msg_storage::{MessageWriter, ReadSpreadRepo};

use runtime::grpc_client::base::service_names as sn;

use crate::kafka;
use crate::pusher::{push_service, Pusher};

// 添加群组服务客户端相关导入
use runtime::grpc_client::base::get_rpc_client;
use contract::proto::group::group_service_client::GroupServiceClient;
use contract::proto::group::GetMembersRequest;
use runtime::service_discovery::LbWithServiceDiscovery;

/// 单批最多并行处理的消息条数
const BATCH_SIZE: usize = 64;

/// 一批从首条消息开始的最长攒批时间
///
/// 原本只按「多久没有新消息」排空，连续流量下会一直攒到 `BATCH_SIZE`：
/// 500 msg/s 时攒满 64 条要 128ms，单条消息平均白等 64ms。
/// 用时间上限同时约束批大小与排队延迟。
const BATCH_MAX_WAIT: Duration = Duration::from_millis(10);

/// 排空批次时等待下一条消息的上限（空闲时尽快处理）
const BATCH_DRAIN_TIMEOUT: Duration = Duration::from_millis(2);

/// 无法解析出会话的消息归到该组
const INVALID_GROUP: &str = "__invalid__";

/// 批量消费时的消息快照
///
/// `BorrowedMessage` 的生命周期绑定在 consumer 上，无法跨 await 持有，
/// 因此先取成拥有所有权的数据再并行处理。
struct PendingMessage {
    payload: String,
    topic: String,
    partition: i32,
    offset: i64,
    /// 分组键：同一会话必须串行，保证序号顺序与消息顺序一致
    group: String,
}

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
/// 4. 将消息存储到数据库（PostgreSQL，读扩散模型）
/// 5. 推送消息到在线用户
/// 6. 处理群聊消息的成员分发
/// 7. 处理已读消息状态更新
///
/// ## 投递语义
///
/// 消费者关闭了自动提交，**只有业务处理成功之后才提交偏移量**，
/// 因此提供的是 at-least-once 而不是 at-most-once：
/// - 处理成功 → 异步提交位点（broker 确认不占用消费关键路径）；
/// - 处理失败 → 有限次重试，仍失败则转入死信主题后再提交，避免阻塞分区；
/// - 死信也投递不出去 → 不提交，让 Kafka 重新投递。
///
/// 代价是可能重复处理。重复处理由 `already_processed` 闸门按 `server_id` 拦掉
/// 已经落库的消息，未落库的消息（通话信令等）本身不参与持久化，重复执行的副作用仅为一次推送。
/// 异步提交若未落到 broker，重启后会重复消费最后一小批；不丢消息，但可能重复推送。
pub struct ConsumerService {
    /// Kafka流式消费者实例
    consumer: StreamConsumer,

    /// 读扩散仓储（会话序号、消息元数据、已读游标）
    read: Arc<ReadSpreadRepo>,

    /// 消息写入器（幂等 → 分配序号 → 写元数据 → 登记成员）
    writer: Arc<MessageWriter>,

    /// 消息推送器
    pusher: Arc<dyn Pusher>,

    /// 缓存接口
    cache: Arc<dyn RelationCache>,

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
        // 先确保主题存在再订阅：Kafka 重启后元数据可能丢失，
        // 若先订阅会在启动瞬间刷 UnknownTopicOrPartition 错误
        kafka::ensure_topic_for(config, &config.kafka.topic)
        .await
        .map_err(|e| Error::Internal(format!("确保Kafka主题存在失败: {}", e)))?;

        let consumer = Self::build_consumer(config)?;

        info!("Kafka消费者已初始化（手动提交），订阅主题: {}", config.kafka.topic);

        // 死信主题与生产者：处理失败的消息最终落到这里，而不是被静默丢弃
        let dead_letter_topic = format!("{}{}", config.kafka.topic, DEAD_LETTER_SUFFIX);
        let dlq_producer = kafka::build_producer(config)?;
        // 死信主题与主主题分区对齐，重放时可按分区一一对应回主主题
        kafka::ensure_topic_for(config, &dead_letter_topic)
        .await
        .map_err(|e| Error::Internal(format!("确保死信主题存在失败: {}", e)))?;

        info!("死信主题已就绪: {}", dead_letter_topic);

        let pusher = push_service(config).await?;
        // 读扩散仓储：独立连接池，避免历史拉取挤占消息写入
        let read = Arc::new(ReadSpreadRepo::from_config(config).await?);
        // 同一 Redis 实例：writer 用 SeqCache，本服务用 RelationCache
        let redis = cache::cache(config).await;
        let seq_cache: Arc<dyn cache::SeqCache> = redis.clone();
        // 号段分配依赖缓存：热群不再对 conversation 行加锁
        let writer = Arc::new(MessageWriter::new(
            read.clone(),
            seq_cache,
            config.message_storage.inline_content_max_bytes,
        ));
        let group_client =
            get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(config, sn::RELATION.to_string())
                .await?;

        info!("消息消费者服务初始化完成");

        Ok(Self {
            consumer,
            read,
            writer,
            pusher,
            cache: redis,
            group_client,
            dlq_producer,
            dead_letter_topic,
            topic: config.kafka.topic.clone(),
        })
    }

    /// 构建并订阅主主题的消费者（关闭自动提交，位点由业务结果决定）
    fn build_consumer(config: &AppConfig) -> Result<StreamConsumer, Error> {
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

        Ok(consumer)
    }

    /// 派生共享依赖、独立订阅的兄弟消费者
    ///
    /// 同一消费组内的多个消费者由 Kafka 自动分配不同分区，
    /// 以此获得跨批次的真并行；每个消费者内部仍保持
    /// 「批内组间并行、组内串行保序」的处理语义。
    /// 复用读扩散仓储、缓存、推送器等重资源，新增开销仅一条消费连接。
    pub async fn fork(&self, config: &AppConfig) -> Result<Self, Error> {
        let consumer = Self::build_consumer(config)?;
        info!("已派生兄弟消费者加入消费组: {}", config.kafka.group);
        Ok(Self {
            consumer,
            read: self.read.clone(),
            writer: self.writer.clone(),
            pusher: self.pusher.clone(),
            cache: self.cache.clone(),
            group_client: self.group_client.clone(),
            dlq_producer: self.dlq_producer.clone(),
            dead_letter_topic: self.dead_letter_topic.clone(),
            topic: self.topic.clone(),
        })
    }

    /// 启动消息消费循环
    ///
    /// 批量拉取 → 按会话分组 → 组内串行（保序）、组间并行 → 整批提交位点。
    /// 同一会话的消息严格串行，保证序号分配顺序与消息实际顺序一致。
    pub async fn consume(self: Arc<Self>) -> Result<(), Error> {
        info!(
            "开始消费Kafka消息: topic={}, 批大小={}, 单条最多尝试 {} 次, 死信主题={}",
            self.topic, BATCH_SIZE, MAX_ATTEMPTS, self.dead_letter_topic
        );

        loop {
            // 阻塞等待首条，避免空转
            let first = match self.consumer.recv().await {
                Ok(message) => message,
                Err(e) => {
                    error!("从Kafka接收消息失败: {}", e);
                    // 接收错误通常是瞬时故障，退避后重试而不是退出服务
                    tokio::time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
            };

            let mut batch = Vec::with_capacity(BATCH_SIZE);
            match Self::pending_from(&first) {
                Some(p) => batch.push(p),
                None => Self::commit(&self.consumer, &first, "负载不可用"),
            }
            drop(first);

            // 大小与时间双重约束：连续流量下最多攒 BATCH_MAX_WAIT，
            // 空闲时等 BATCH_DRAIN_TIMEOUT 即处理，避免低速率消息长队等待
            let batch_deadline = Instant::now() + BATCH_MAX_WAIT;
            while batch.len() < BATCH_SIZE {
                let remaining = batch_deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(
                    remaining.min(BATCH_DRAIN_TIMEOUT),
                    self.consumer.recv(),
                )
                .await
                {
                    Ok(Ok(message)) => match Self::pending_from(&message) {
                        Some(p) => batch.push(p),
                        None => Self::commit(&self.consumer, &message, "负载不可用"),
                    },
                    _ => break,
                }
            }

            if !batch.is_empty() {
                self.process_batch(batch).await;
            }
        }
    }

    /// 把 Kafka 消息取成拥有所有权的快照
    ///
    /// 同时提前解析会话ID，作为「同会话保序」的分组键。
    fn pending_from(message: &BorrowedMessage<'_>) -> Option<PendingMessage> {
        let payload = message.payload_view::<str>()?.ok()?.to_owned();
        let group = Self::group_of_payload(&payload);

        Some(PendingMessage {
            payload,
            topic: message.topic().to_string(),
            partition: message.partition(),
            offset: message.offset(),
            group,
        })
    }

    /// 从 Kafka 负载解析出会话分组键
    ///
    /// 外层信封由生产端用 serde_json 序列化，这里必须按 JSON 解析；
    /// 只有 `content` 字段内部才是 bincode。解析失败只损失并行度
    /// （全部落到同一分组串行处理），不影响业务正确性。
    fn group_of_payload(payload: &str) -> String {
        match serde_json::from_str::<Msg>(payload) {
            Ok(msg) => conversation_id_of(&msg),
            Err(_) => INVALID_GROUP.to_string(),
        }
    }

    /// 并行处理一批消息，并按分区提交位点
    async fn process_batch(self: &Arc<Self>, batch: Vec<PendingMessage>) {
        // 批内每个分区的最大位点
        let mut max_offsets: std::collections::HashMap<(String, i32), i64> =
            std::collections::HashMap::new();
        for m in &batch {
            let entry = max_offsets
                .entry((m.topic.clone(), m.partition))
                .or_insert(m.offset);
            if m.offset > *entry {
                *entry = m.offset;
            }
        }

        // 按会话分组：组内串行保序，组间并行
        let mut groups: std::collections::HashMap<String, Vec<PendingMessage>> =
            std::collections::HashMap::new();
        for m in batch {
            groups.entry(m.group.clone()).or_default().push(m);
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PendingMessage>();

        let mut handles = Vec::with_capacity(groups.len());
        for (_, msgs) in groups {
            let svc = self.clone();
            let tx = tx.clone();

            handles.push(tokio::spawn(async move {
                for m in msgs {
                    let payload = m.payload.clone();
                    let result =
                        retry_with_backoff(MAX_ATTEMPTS, RETRY_BACKOFF, || svc.handle_msg(&payload))
                            .await;

                    if let Err(e) = result {
                        error!("消息处理失败且重试耗尽: {:?}", e);
                        // 优先转死信；死信也写不进去时，把该分区标记为需要回退
                        if let Err(dlq_err) = kafka::send_to_topic(
                            &svc.dlq_producer,
                            &svc.dead_letter_topic,
                            &m.payload,
                            &m.group,
                        )
                        .await
                        {
                            warn!("死信投递失败({})，回退偏移量等待重新投递", dlq_err);
                            let _ = tx.send(m);
                        }
                    }
                }
            }));
        }
        drop(tx);

        for handle in handles {
            if let Err(e) = handle.await {
                error!("批处理任务异常退出: {:?}", e);
            }
        }

        // 收集需要回退的分区及最小位点
        let mut rollback: std::collections::HashMap<(String, i32), i64> =
            std::collections::HashMap::new();
        while let Some(m) = rx.recv().await {
            let key = (m.topic, m.partition);
            let entry = rollback.entry(key).or_insert(m.offset);
            if m.offset < *entry {
                *entry = m.offset;
            }
        }

        let mut tpl = rdkafka::TopicPartitionList::new();
        for ((topic, partition), offset) in max_offsets {
            if let Some(&seek_to) = rollback.get(&(topic.clone(), partition)) {
                // 有消息没能落库，该分区不提交，退回重新投递。
                // 已处理过的消息会重复消费，由幂等闸门兜底。
                if let Err(e) = self.consumer.seek(
                    &topic,
                    partition,
                    Offset::Offset(seek_to),
                    Duration::from_secs(1),
                ) {
                    error!("回退偏移量 {} 失败: {:?}", seek_to, e);
                }
                continue;
            }

            if let Err(e) = tpl.add_partition_offset(&topic, partition, Offset::Offset(offset + 1))
            {
                error!("构造提交位点失败: {:?}", e);
            }
        }

        if !tpl.elements().is_empty() {
            // 异步提交：位点确认不占用消费循环。提交丢失只会让重启后重复消费
            // 最后一小批，由 already_processed 闸门兜底，不会丢消息。
            match self.consumer.commit(&tpl, CommitMode::Async) {
                Ok(()) => debug!("已异步提交 {} 个分区的位点", tpl.elements().len()),
                Err(e) => error!("提交偏移量失败: {:?}", e),
            }
        }
    }

    /// 异步提交无效负载的位点，避免污染消费循环的延迟
    fn commit(consumer: &StreamConsumer, message: &BorrowedMessage<'_>, reason: &str) {
        if let Err(e) = consumer.commit_message(message, CommitMode::Async) {
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
        if already_processed(&*self.read.meta, &msg).await? {
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

        let (msg_type, _need_increase_seq, need_history) = MsgType2::classify_msg_type(mt);
        debug!("消息分类结果: {:?}, 需要历史: {}", msg_type, need_history);

        // 群成员（不含发送者），用于登记 user_conversation 与推送路由
        let member_ids = self.group_member_ids(&msg_type, &msg).await?;

        // 读扩散：一条消息只写一次。序号是会话级的，与成员数量无关。
        if Self::get_send_to_db_flag(&mt) && !msg.is_revoked && need_history {
            let mut participants = member_ids.clone();
            if !msg.send_id.is_empty() {
                participants.push(msg.send_id.clone());
            }
            // 单聊的另一方不在 member_ids 里，需显式补上
            if msg_type != MsgType2::Group && !msg.receiver_id.is_empty() {
                participants.push(msg.receiver_id.clone());
            }

            let (conversation_id, seq, inserted) =
                self.writer.save(msg.clone(), &participants).await?;
            msg.seq = seq;
            debug!(
                "会话 {} 分配序号 {}（新写入: {}）server_id={}",
                conversation_id, seq, inserted, msg.server_id
            );
        }

        // 推送使用会话级序号：同一会话的所有成员看到同一个 seq
        let members: Vec<GroupMemSeq> = member_ids
            .into_iter()
            .map(|mem_id| GroupMemSeq {
                mem_id,
                cur_seq: msg.seq,
                max_seq: msg.seq,
                need_update: false,
            })
            .collect();

        let mut tasks = Vec::with_capacity(1);

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

    /// 处理消息已读状态更新
    ///
    /// 读扩散下已读是会话级游标，只写 1 行，与群成员数无关。
    async fn handle_msg_read(&self, msg: Msg) -> Result<(), Error> {
        let data: MsgRead = bincode::deserialize(&msg.content)
            .map_err(|_| Error::Internal("反序列化MsgRead失败".to_string()))?;

        if data.conversation_id.is_empty() {
            // 老客户端仍按 msg_seq 逐条上报，读扩散下无法映射到会话，只能忽略
            warn!(
                "已读消息缺少 conversation_id，已忽略: user_id={}",
                data.user_id
            );
            return Ok(());
        }

        self.read
            .user_conversation
            .mark_read(&data.user_id, &data.conversation_id, data.read_seq)
            .await?;
        Ok(())
    }

    /// 处理群聊消息的序列号分配
    ///
    /// 群解散、成员退出等操作会顺带清理缓存中的群成员信息。
    async fn group_member_ids(
        &self,
        msg_type: &MsgType2,
        msg: &Msg,
    ) -> Result<Vec<String>, Error> {
        if *msg_type != MsgType2::Group {
            return Ok(vec![]);
        }

        let mut members = self.get_members_id(&msg.receiver_id).await?;

        // 排除发送者自己（发送者不需要接收自己的消息）
        members.retain(|id| id != &msg.send_id);

        Ok(members)
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

}

/// 幂等闸门：判断一条消息是否已经落过库
///
/// 以 `server_id` 为准 —— 它在生产端由 `nanoid` 生成，是全局唯一且稳定的。
pub(crate) async fn already_processed(
    repo: &dyn MsgMetaRepo,
    msg: &Msg,
) -> Result<bool, Error> {
    if msg.server_id.is_empty() {
        // 历史数据可能存在空 ID，无法据此判定重复，放行交给后续流程
        return Ok(false);
    }
    // 幂等键是 (conversation_id, server_id)，与读扩散的唯一约束一致
    let conversation_id = conversation_id_of(msg);
    Ok(repo.get(&conversation_id, &msg.server_id).await?.is_some())
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
    use contract::proto::message::{Msg, MsgType};
    use msg_storage::read_spread::MsgEntry;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn payload_group_uses_the_shared_conversation_key() {
        // 跨 crate 契约：Kafka JSON 回读后的分组键，必须与发送端分区键、
        // 网关上行保序键来自同一推导（conversation_id_of）
        let single = Msg {
            send_id: "u2".into(),
            receiver_id: "u1".into(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        };
        let group = Msg {
            send_id: "u2".into(),
            group_id: "g1".into(),
            msg_type: MsgType::GroupMsg as i32,
            ..Default::default()
        };
        for msg in [single, group] {
            let payload = serde_json::to_string(&msg).unwrap();
            assert_eq!(ConsumerService::group_of_payload(&payload), conversation_id_of(&msg));
        }
    }

    /// 只实现幂等闸门用得到的 `get`，其余方法不应被调用
    #[derive(Debug)]
    struct FakeMeta {
        existing: Option<Msg>,
        fail: bool,
    }

    #[async_trait]
    impl MsgMetaRepo for FakeMeta {
        async fn insert(&self, _entry: &MsgEntry, _inline_max_bytes: usize) -> Result<bool, Error> {
            unimplemented!()
        }

        async fn insert_batch(
            &self,
            _entries: &[MsgEntry],
            _inline_max_bytes: usize,
        ) -> Result<u64, Error> {
            unimplemented!()
        }

        async fn fetch_after(
            &self,
            _conversation_id: &str,
            _after_seq: i64,
            _limit: i64,
        ) -> Result<Vec<Msg>, Error> {
            unimplemented!()
        }

        async fn fetch_range(
            &self,
            _conversation_id: &str,
            _start_seq: i64,
            _end_seq: i64,
        ) -> Result<Vec<Msg>, Error> {
            unimplemented!()
        }

        async fn get(
            &self,
            _conversation_id: &str,
            _server_id: &str,
        ) -> Result<Option<Msg>, Error> {
            if self.fail {
                return Err(Error::Internal("数据库不可用".to_string()));
            }
            Ok(self.existing.clone())
        }

        async fn revoke(
            &self,
            _conversation_id: &str,
            _seq: i64,
            _revoked_by: &str,
            _revoke_time: i64,
        ) -> Result<(), Error> {
            unimplemented!()
        }

        async fn delete_by_server_id(
            &self,
            _conversation_id: &str,
            _server_ids: &[String],
        ) -> Result<u64, Error> {
            unimplemented!()
        }

        async fn delete_conversation(&self, _conversation_id: &str) -> Result<u64, Error> {
            unimplemented!()
        }
    }

    fn test_msg(server_id: &str) -> Msg {
        Msg {
            server_id: server_id.into(),
            send_id: "u1".into(),
            receiver_id: "u2".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn duplicate_message_is_rejected_by_gate() {
        let repo = FakeMeta {
            existing: Some(test_msg("s1")),
            fail: false,
        };

        assert!(
            already_processed(&repo, &test_msg("s1")).await.unwrap(),
            "已落库的消息必须被判定为重复，避免位点回退后重复计数"
        );
    }

    #[tokio::test]
    async fn fresh_message_passes_gate() {
        let repo = FakeMeta {
            existing: None,
            fail: false,
        };

        assert!(!already_processed(&repo, &test_msg("s1")).await.unwrap());
    }

    #[tokio::test]
    async fn empty_server_id_is_never_treated_as_duplicate() {
        let repo = FakeMeta {
            existing: Some(Msg::default()),
            fail: false,
        };

        // 空 ID 无法判定，放行，否则历史脏数据会被永久卡住
        assert!(!already_processed(&repo, &test_msg("")).await.unwrap());
    }

    #[tokio::test]
    async fn gate_propagates_storage_error() {
        let repo = FakeMeta {
            existing: None,
            fail: true,
        };

        // 判定不出来就不能当作「没处理过」，否则等于放弃了幂等保护
        assert!(already_processed(&repo, &test_msg("s1")).await.is_err());
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

    #[test]
    fn group_of_single_chat_payload_is_json_parsed() {
        // 生产端用 serde_json 写 Kafka，分组键也必须按 JSON 解析，
        // 否则所有消息都会落到 __invalid__，退化成全串行
        let msg = Msg {
            send_id: "u2".into(),
            receiver_id: "u1".into(),
            msg_type: MsgType::SingleMsg as i32,
            ..Default::default()
        };
        let payload = serde_json::to_string(&msg).unwrap();

        assert_eq!(ConsumerService::group_of_payload(&payload), "d:u1:u2");
    }

    #[test]
    fn group_of_group_msg_prefers_group_id() {
        let msg = Msg {
            send_id: "u1".into(),
            receiver_id: "g1".into(),
            group_id: "g1".into(),
            msg_type: MsgType::GroupMsg as i32,
            ..Default::default()
        };
        let payload = serde_json::to_string(&msg).unwrap();

        assert_eq!(ConsumerService::group_of_payload(&payload), "g:g1");
    }

    #[test]
    fn group_of_broken_payload_falls_back_to_invalid() {
        assert_eq!(
            ConsumerService::group_of_payload("not-a-json-envelope"),
            INVALID_GROUP
        );
    }
}
