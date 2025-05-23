use std::sync::Arc;

use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::{ClientConfig, Message};
use tracing::{debug, error, info, warn};

use cache::Cache;
use common::config::AppConfig;
use common::error::Error;
use common::message::{GroupMemSeq, Msg, MsgRead, MsgType};
use msg_storage::{msg_rec_box_repo, DbRepo};
use msg_storage::message::MsgRecBoxRepo;
use crate::pusher::{push_service, Pusher};

/// 消息类型的简化枚举
/// 用于内部区分单聊和群聊消息的处理逻辑
#[derive(Debug, Clone, Eq, PartialEq)]
enum MsgType2 {
    // 单聊消息
    Single,
    // 群聊消息
    Group,
}

/// 消息消费者服务
/// 负责从Kafka消费消息，处理消息，并分发到各个目标
pub struct ConsumerService {
    // Kafka消费者实例
    consumer: StreamConsumer,
    // 数据库操作封装
    db: Arc<DbRepo>,
    // 消息盒子仓库，用于存储离线消息
    msg_box: Arc<dyn MsgRecBoxRepo>,
    // 消息推送器，用于将消息推送给客户端
    pusher: Arc<dyn Pusher>,
    // 缓存接口，用于存取高频访问数据
    cache: Arc<dyn Cache>,
    // 序列号步长，用于生成消息序列号
    seq_step: i32,
}

impl ConsumerService {
    /// 创建新的消费者服务实例
    /// 初始化Kafka消费者、数据库连接、缓存等组件
    pub async fn new(config: &AppConfig) -> Result<Self, Error> {
        // 创建Kafka消费者配置
        let consumer: StreamConsumer = ClientConfig::new()
            .set("group.id", &config.kafka.group)
            .set("bootstrap.servers", config.kafka.hosts.join(","))
            .set("enable.partition.eof", "false")
            .set("session.timeout.ms", config.kafka.consumer.session_timeout.to_string())
            .set("enable.auto.commit", "true")
            .set("auto.offset.reset", &config.kafka.consumer.auto_offset_reset)
            .create()
            .map_err(|e| Error::Internal(format!("消费者创建失败: {}", e)))?;

        // 订阅指定的Kafka主题
        consumer
            .subscribe(&[&config.kafka.topic])
            .map_err(|e| Error::Internal(format!("无法订阅指定的主题: {}", e)))?;

        // 初始化推送服务
        let pusher = push_service(config).await?;
        // 初始化数据库仓库
        let db = Arc::new(DbRepo::new(config).await);

        // 获取序列号步长配置
        let seq_step = config.redis.seq_step;

        // 初始化缓存和消息盒子仓库
        let cache = cache::cache(config).await;
        let msg_box = msg_rec_box_repo(config).await?;

        Ok(Self {
            consumer,
            db,
            msg_box,
            pusher,
            cache,
            seq_step,
        })
    }

    /// 启动消息消费循环
    /// 不断从Kafka获取消息并处理
    pub async fn consume(&mut self) -> Result<(), Error> {
        loop {
            match self.consumer.recv().await {
                Err(e) => error!("Kafka错误: {}", e),
                Ok(m) => {
                    // 尝试获取消息内容并处理
                    if let Some(Ok(payload)) = m.payload_view::<str>() {
                        if let Err(e) = self.handle_msg(payload).await {
                            error!("处理消息失败: {:?}", e);
                            continue;
                        }
                        // 异步提交消息偏移量，确认消息已处理
                        if let Err(e) = self.consumer.commit_message(&m, CommitMode::Async) {
                            error!("提交消息偏移量失败: {:?}", e);
                        }
                    }
                }
            }
        }
    }

    /// 处理单条消息的核心逻辑
    /// 解析消息内容，根据类型进行不同处理
    async fn handle_msg(&self, payload: &str) -> Result<(), Error> {
        debug!("收到消息: {:#?}", payload);

        // 将JSON字符串解析为消息对象
        let mut msg: Msg = serde_json::from_str(payload)?;

        // 将整数类型转换为枚举类型，便于处理
        let mt = MsgType::try_from(msg.msg_type).map_err(|e| Error::Internal(e.to_string()))?;

        // 处理已读类型的消息，这类消息有特殊的处理逻辑
        if mt == MsgType::Read {
            return self.handle_msg_read(msg).await;
        }

        // 根据消息类型进行分类，确定处理策略
        let (msg_type, need_increase_seq, need_history) = self.classify_msg_type(mt).await;

        // 检查发送者序列号，如果需要则增加最大序列号
        self.handle_send_seq(&msg.send_id).await?;

        // 处理接收者序列号
        if need_increase_seq {
            // 为消息分配一个新的序列号
            let cur_seq = self.increase_message_seq(&msg.receiver_id).await?;
            msg.seq = cur_seq;
        }

        // 如果是群聊消息，查询群成员ID并处理群聊序列号
        let members = self.handle_group_seq(&msg_type, &mut msg).await?;

        // 创建任务集合，包含数据库存储和消息推送
        let mut tasks = Vec::with_capacity(2);
        
        // 判断是否需要发送到数据库
        if Self::get_send_to_db_flag(&mt) {
            let cloned_msg = msg.clone();
            let cloned_type = msg_type.clone();
            let cloned_members = members.clone();
            
            // 克隆数据库和消息盒子引用用于异步任务
            let db = self.db.clone();
            let msg_box = self.msg_box.clone();
            
            // 创建发送到数据库的异步任务
            let to_db = tokio::spawn(async move {
                if let Err(e) = Self::send_to_db(
                    db,
                    msg_box,
                    cloned_msg,
                    cloned_type,
                    need_history,
                    cloned_members,
                )
                .await
                {
                    error!("发送消息到数据库失败: {:?}", e);
                }
            });

            tasks.push(to_db);
        }

        // 创建发送到推送服务的异步任务
        let pusher = self.pusher.clone();
        let to_pusher = tokio::spawn(async move {
            match msg_type {
                // 处理单聊消息推送
                MsgType2::Single => {
                    if let Err(e) = pusher.push_single_msg(msg).await {
                        error!("发送消息到推送服务失败: {:?}", e);
                    }
                }
                // 处理群聊消息推送
                MsgType2::Group => {
                    if let Err(e) = pusher.push_group_msg(msg, members).await {
                        error!("发送消息到推送服务失败: {:?}", e);
                    }
                }
            }
        });
        tasks.push(to_pusher);

        // 等待所有任务完成
        futures::future::try_join_all(tasks)
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;

        Ok(())
    }

    /// 根据消息类型分类，确定处理策略
    /// 返回值: (消息类型, 是否需要增加序列号, 是否需要存储历史记录)
    async fn classify_msg_type(&self, mt: MsgType) -> (MsgType2, bool, bool) {
        let msg_type;
        let mut need_increase_seq = false;
        let mut need_history = true;

        match mt {
            // 单聊消息类型，需要增加序列号
            MsgType::SingleMsg
            | MsgType::SingleCallInviteNotAnswer
            | MsgType::SingleCallInviteCancel
            | MsgType::Hangup
            | MsgType::ConnectSingleCall
            | MsgType::RejectSingleCall
            | MsgType::FriendApplyReq
            | MsgType::FriendApplyResp
            | MsgType::FriendDelete => {
                // 单聊消息，需要增加序列号
                msg_type = MsgType2::Single;
                need_increase_seq = true;
            }
            // 群聊消息类型，序列号处理方式特殊
            MsgType::GroupMsg => {
                // 群聊消息，需要增加每个成员的序列号
                // 但不是在这里处理，而是在handle_group_seq中处理
                msg_type = MsgType2::Group;
            }
            // 其他消息类型...
            MsgType::GroupInvitation
            | MsgType::GroupInviteNew
            | MsgType::GroupMemberExit
            | MsgType::GroupRemoveMember
            | MsgType::GroupDismiss
            | MsgType::GroupUpdate => {
                // group message and need to increase seq
                msg_type = MsgType2::Group;
                need_history = false;
            }
            // single call data exchange and don't need to increase seq
            MsgType::GroupDismissOrExitReceived
            | MsgType::GroupInvitationReceived
            | MsgType::FriendBlack
            | MsgType::SingleCallInvite
            | MsgType::AgreeSingleCall
            | MsgType::SingleCallOffer
            | MsgType::Candidate
            | MsgType::Read
            | MsgType::MsgRecResp
            | MsgType::Notification
            | MsgType::Service
            | MsgType::FriendshipReceived => {
                msg_type = MsgType2::Single;
                need_history = false;
            }
        }
        
        return (msg_type, need_increase_seq, need_history);
    }

    /// query members id from cache
    /// if not found, query from db
    async fn get_members_id(&self, group_id: &str) -> Result<Vec<String>, Error> {
        match self.cache.query_group_members_id(group_id).await {
            Ok(list) if !list.is_empty() => Ok(list),
            Ok(_) => {
                warn!("group members id is empty from cache");
                // query from db
                self.query_group_members_id_from_db(group_id).await
            }
            Err(err) => {
                error!("failed to query group members id from cache: {:?}", err);
                Err(err)
            }
        }
    }

    async fn handle_send_seq(&self, user_id: &str) -> Result<(), Error> {
        let send_seq = self.cache.get_send_seq(user_id).await?;

        if send_seq.0 == send_seq.1 - self.seq_step as i64 {
            self.db.seq.save_max_seq(user_id).await?;
        }
        Ok(())
    }

    async fn increase_message_seq(&self, user_id: &str) -> Result<i64, Error> {
        let (cur_seq, _, updated) = self.cache.increase_seq(user_id).await?;
        if updated {
            self.db.seq.save_max_seq(user_id).await?;
        }
        Ok(cur_seq)
    }

    async fn handle_msg_read(&self, msg: Msg) -> Result<(), Error> {
        let data: MsgRead = bincode::deserialize(&msg.content).map_err(|_| Error::Internal("failed to deserialize MsgRead".to_string()))?;

        self.msg_box.msg_read(&data.user_id, &data.msg_seq).await?;
        Ok(())
    }

    async fn handle_group_seq(
        &self,
        msg_type: &MsgType2,
        msg: &mut Msg,
    ) -> Result<Vec<GroupMemSeq>, Error> {
        if *msg_type != MsgType2::Group {
            return Ok(vec![]);
        }
        // query group members id from the cache
        let mut members = self.get_members_id(&msg.receiver_id).await?;

        // retain the members id
        members.retain(|id| id != &msg.send_id);

        // increase the members seq
        let seq = self.cache.incr_group_seq(members).await?;

        // we should send the whole list to db module and db module will handle the data

        // judge the message type;
        // we should delete the cache data if the type is group dismiss
        // update the cache if the type is group member exit
        if msg.msg_type == MsgType::GroupDismiss as i32 {
            self.cache.del_group_members(&msg.receiver_id).await?;
        } else if msg.msg_type == MsgType::GroupMemberExit as i32 {
            self.cache
                .remove_group_member_id(&msg.receiver_id, &msg.send_id)
                .await?;
        } else if msg.msg_type == MsgType::GroupRemoveMember as i32 {
            let data: Vec<String> =
                bincode::deserialize(&msg.content).map_err(|e| Error::Internal(e.to_string()))?;

            let member_ids_ref: Vec<&str> = data.iter().map(AsRef::as_ref).collect();
            self.cache
                .remove_group_member_batch(&msg.group_id, &member_ids_ref)
                .await?;
        }

        Ok(seq)
    }

    /// there is no need to send to db
    /// if the message type is related to call protocol
    #[inline]
    fn get_send_to_db_flag(msg_type: &MsgType) -> bool {
        !matches!(
            *msg_type,
            MsgType::ConnectSingleCall
                | MsgType::AgreeSingleCall
                | MsgType::Candidate
                | MsgType::SingleCallOffer
                | MsgType::SingleCallInvite
        )
    }

    async fn send_to_db(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        msg: Msg,
        msg_type: MsgType2,
        need_to_history: bool,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        // match the message type to procedure the different method
        match msg_type {
            MsgType2::Single => {
                Self::handle_message(db, msg_box, msg, need_to_history).await?;
            }
            MsgType2::Group => {
                Self::handle_group_message(db, msg_box, msg, need_to_history, members).await?;
            }
        }

        Ok(())
    }

    /// query members id from database
    /// and set it to cache
    async fn query_group_members_id_from_db(&self, group_id: &str) -> Result<Vec<String>, Error> {
        /// TODO query members id from database
        // let members_id = self.db.group.query_group_members_id(group_id).await?;
        let members_id = Vec::new();

        // save it to cache
        if let Err(e) = self
            .cache
            .save_group_members_id(group_id, members_id.clone())
            .await
        {
            error!("failed to save group members id to cache: {:?}", e);
        }

        Ok(members_id)
    }

    async fn handle_message(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        message: Msg,
        need_to_history: bool,
    ) -> Result<(), Error> {
        // task 1 save message to postgres

        let mut tasks = Vec::with_capacity(2);
        if !need_to_history {
            let cloned_msg = message.clone();
            let db_task = tokio::spawn(async move {
                if let Err(e) = db.msg.save_message(cloned_msg).await {
                    tracing::error!("save message to db failed: {}", e);
                }
            });
            tasks.push(db_task);
        }

        // task 2 save message to mongodb
        let msg_rec_box_task = tokio::spawn(async move {
            // if the message type is friendship/group-operation delivery, we should delete it from mongodb
            if message.msg_type == MsgType::GroupDismissOrExitReceived as i32
                || message.msg_type == MsgType::GroupInvitationReceived as i32
                || message.msg_type == MsgType::FriendshipReceived as i32
            {
                if let Err(e) = msg_box.delete_message(&message.server_id).await {
                    tracing::error!("delete message from mongodb failed: {}", e);
                }
                return;
            }
            if let Err(e) = msg_box.save_message(&message).await {
                tracing::error!("save message to mongodb failed: {}", e);
            }
        });
        tasks.push(msg_rec_box_task);

        // wait all tasks
        futures::future::try_join_all(tasks)
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(())
    }

    async fn handle_group_message(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        message: Msg,
        need_to_history: bool,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        // task 1 save message to postgres
        // update the user's seq in postgres
        let need_update = members
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if item.need_update {
                    members.get(index).map(|v| v.mem_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<String>>();

        let cloned_msg = if need_to_history {
            Some(message.clone())
        } else {
            None
        };

        let db_task = tokio::spawn(async move {
            if !need_update.is_empty() {
                if let Err(err) = db.seq.save_max_seq_batch(&need_update).await {
                    tracing::error!("save max seq batch failed: {}", err);
                    return Err(err);
                };
            }

            if let Some(cloned_msg) = cloned_msg {
                if let Err(e) = db.msg.save_message(cloned_msg).await {
                    tracing::error!("save message to db failed: {}", e);
                    return Err(e);
                }
            }
            Ok(())
        });

        // task 2 save message to mongodb
        let msg_rec_box_task = tokio::spawn(async move {
            if let Err(e) = msg_box.save_group_msg(message, members).await {
                tracing::error!("save message to mongodb failed: {}", e);
                return Err(e);
            }
            Ok(())
        });

        // wait all tasks complete
        let (db_result, msg_rec_box_result) = tokio::try_join!(db_task, msg_rec_box_task)
            .map_err(|e| Error::Internal(e.to_string()))?;

        db_result?;
        msg_rec_box_result?;

        Ok(())
    }
}
