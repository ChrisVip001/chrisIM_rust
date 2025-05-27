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

/// 消息类型的简化分类枚举
/// 
/// 为了简化消息处理逻辑，将复杂的消息类型归类为两种基本类型：
/// - 单聊消息：点对点的私人消息
/// - 群聊消息：一对多的群组消息
/// 
/// 这种分类有助于统一处理流程，避免代码重复
#[derive(Debug, Clone, Eq, PartialEq)]
enum MsgType2 {
    /// 单聊消息类型
    /// 包括普通文本、图片、语音、视频等私人消息
    Single,
    
    /// 群聊消息类型  
    /// 包括群聊文本、群公告、群成员变更等群组消息
    Group,
}

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
/// ## 工作流程
/// 
/// ```text
/// Kafka队列 -> 消费消息 -> 解析消息 -> 分配序列号 -> 并行处理:
///                                                  ├─ 存储到数据库
///                                                  └─ 推送给用户
/// ```
pub struct ConsumerService {
    /// Kafka流式消费者实例
    /// 配置了消费者组、偏移量管理等参数
    consumer: StreamConsumer,
    
    /// 数据库操作仓库
    /// 封装了PostgreSQL的消息存储和序列号管理操作
    db: Arc<DbRepo>,
    
    /// MongoDB消息盒子仓库
    /// 用于存储用户的离线消息，支持消息查询和删除
    msg_box: Arc<dyn MsgRecBoxRepo>,
    
    /// 消息推送器
    /// 负责将消息实时推送到在线用户的WebSocket连接
    pusher: Arc<dyn Pusher>,
    
    /// 缓存接口
    /// 用于缓存用户序列号、在线状态等高频访问数据
    cache: Arc<dyn Cache>,
    
    /// 序列号增长步长
    /// 每次为用户分配新序列号时的增量，用于性能优化
    seq_step: i32,
}

impl ConsumerService {
    /// 创建新的消息消费者服务实例
    /// 
    /// 初始化所有必要的组件：
    /// - Kafka消费者客户端
    /// - 数据库连接池
    /// - 缓存连接
    /// - 消息推送服务
    /// - MongoDB消息盒子
    /// 
    /// # 参数
    /// * `config` - 应用程序配置，包含所有服务的配置信息
    /// 
    /// # 返回值
    /// * `Ok(ConsumerService)` - 成功创建的消费者服务实例
    /// * `Err(Error)` - 初始化失败的错误信息
    pub async fn new(config: &AppConfig) -> Result<Self, Error> {
        // 创建并配置Kafka消费者
        // 设置消费者组、自动提交、会话超时等关键参数
        let consumer: StreamConsumer = ClientConfig::new()
            .set("group.id", &config.kafka.group)                                    // 消费者组ID
            .set("bootstrap.servers", config.kafka.hosts.join(","))                 // Kafka集群地址
            .set("enable.partition.eof", "false")                                    // 不发送分区结束标记
            .set("session.timeout.ms", config.kafka.consumer.session_timeout.to_string()) // 会话超时时间
            .set("enable.auto.commit", "true")                                       // 启用自动提交偏移量
            .set("auto.offset.reset", &config.kafka.consumer.auto_offset_reset)     // 偏移量重置策略
            .create()
            .map_err(|e| Error::Internal(format!("Kafka消费者创建失败: {}", e)))?;

        // 订阅指定的Kafka主题
        // 消费者将从这个主题接收所有消息
        consumer
            .subscribe(&[&config.kafka.topic])
            .map_err(|e| Error::Internal(format!("无法订阅Kafka主题 '{}': {}", config.kafka.topic, e)))?;

        info!("Kafka消费者已初始化，订阅主题: {}", config.kafka.topic);

        // 初始化消息推送服务
        // 根据配置创建合适的推送器（WebSocket、HTTP等）
        let pusher = push_service(config).await?;
        
        // 初始化数据库仓库
        // 包含PostgreSQL连接池和序列号管理
        let db = Arc::new(DbRepo::new(config).await);

        // 获取序列号步长配置
        // 用于批量分配序列号，提高性能
        let seq_step = config.redis.seq_step;

        // 初始化Redis缓存连接
        // 用于缓存用户状态和序列号信息
        let cache = cache::cache(config).await;
        
        // 初始化MongoDB消息盒子仓库
        // 用于存储离线消息和消息查询
        let msg_box = msg_rec_box_repo(config).await?;

        info!("消息消费者服务初始化完成");
        
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
    /// 
    /// 这是服务的主要工作方法，会无限循环地：
    /// 1. 从Kafka接收消息
    /// 2. 解析和处理消息内容  
    /// 3. 提交消息偏移量确认处理完成
    /// 4. 处理异常情况并记录日志
    /// 
    /// 该方法会一直运行直到服务停止或发生致命错误。
    /// 
    /// # 返回值
    /// * `Ok(())` - 正常情况下不会返回，除非服务停止
    /// * `Err(Error)` - 发生致命错误导致服务无法继续
    pub async fn consume(&mut self) -> Result<(), Error> {
        info!("开始消费Kafka消息...");
        
        loop {
            match self.consumer.recv().await {
                Err(e) => {
                    error!("从Kafka接收消息失败: {}", e);
                    // 遇到接收错误时继续循环，不中断服务
                    continue;
                }
                Ok(m) => {
                    // 尝试获取消息负载内容并处理
                    if let Some(Ok(payload)) = m.payload_view::<str>() {
                        debug!("收到Kafka消息，长度: {} 字节", payload.len());
                        
                        // 处理消息内容
                        if let Err(e) = self.handle_msg(payload).await {
                            error!("处理消息失败: {:?}, 消息内容: {}", e, payload);
                            // 即使处理失败也要提交偏移量，避免重复消费
                        }
                        
                        // 异步提交消息偏移量，确认消息已被处理
                        // 使用异步模式提高性能，不等待确认
                        if let Err(e) = self.consumer.commit_message(&m, CommitMode::Async) {
                            error!("提交Kafka消息偏移量失败: {:?}", e);
                        }
                    } else {
                        warn!("收到空消息或消息解码失败");
                    }
                }
            }
        }
    }

    /// 处理单条消息的核心业务逻辑
    /// 
    /// 这是消息处理的核心方法，执行完整的消息处理流程：
    /// 1. 解析JSON格式的消息
    /// 2. 分类消息类型（单聊/群聊/系统消息）
    /// 3. 处理序列号分配
    /// 4. 并行执行数据库存储和消息推送
    /// 
    /// # 参数
    /// * `payload` - 从Kafka接收到的JSON格式消息字符串
    /// 
    /// # 返回值
    /// * `Ok(())` - 消息处理成功
    /// * `Err(Error)` - 消息处理失败，包含详细错误信息
    async fn handle_msg(&self, payload: &str) -> Result<(), Error> {
        debug!("开始处理消息: {}", payload);

        // 将JSON字符串反序列化为消息对象
        let mut msg: Msg = serde_json::from_str(payload)
            .map_err(|e| Error::Internal(format!("消息JSON解析失败: {}", e)))?;

        // 将消息类型从整数转换为枚举，便于后续处理
        let mt = MsgType::try_from(msg.msg_type)
            .map_err(|e| Error::Internal(format!("未知的消息类型 {}: {}", msg.msg_type, e)))?;

        // 特殊处理已读消息类型
        // 已读消息有独立的处理逻辑，不需要序列号和存储
        if mt == MsgType::Read {
            debug!("处理已读消息: user_id={}", msg.receiver_id);
            return self.handle_msg_read(msg).await;
        }

        // 根据消息类型进行分类，确定处理策略
        let (msg_type, need_increase_seq, need_history) = self.classify_msg_type(mt).await;
        debug!("消息分类结果: {:?}, 需要序列号: {}, 需要历史: {}", 
               msg_type, need_increase_seq, need_history);

        // 处理发送者的发送序列号
        // 每个用户都有自己的发送序列号，用于消息去重和排序
        self.handle_send_seq(&msg.send_id).await?;

        // 处理接收者序列号
        if need_increase_seq {
            // 为消息分配一个新的接收序列号
            let cur_seq = self.increase_message_seq(&msg.receiver_id).await?;
            msg.seq = cur_seq;
            debug!("为用户 {} 分配序列号: {}", msg.receiver_id, cur_seq);
        }

        // 如果是群聊消息，需要查询群成员并处理群聊序列号
        let members = self.handle_group_seq(&msg_type, &mut msg).await?;

        // 创建并行任务列表
        // 同时执行数据库存储和消息推送，提高处理效率
        let mut tasks = Vec::with_capacity(2);
        
        // 判断是否需要存储到数据库
        if Self::get_send_to_db_flag(&mt) {
            let cloned_msg = msg.clone();
            let cloned_type = msg_type.clone();
            let cloned_members = members.clone();
            
            // 克隆必要的引用用于异步任务
            let db = self.db.clone();
            let msg_box = self.msg_box.clone();
            
            // 创建数据库存储异步任务
            let to_db = tokio::spawn(async move {
                debug!("开始存储消息到数据库: server_id={}", cloned_msg.server_id);
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
                    error!("存储消息到数据库失败: {:?}", e);
                }
            });

            tasks.push(to_db);
        }

        // 创建消息推送异步任务
        let pusher = self.pusher.clone();
        let to_pusher = tokio::spawn(async move {
            debug!("开始推送消息: server_id={}, 类型={:?}", msg.server_id, msg_type);
            match msg_type {
                // 处理单聊消息推送
                MsgType2::Single => {
                    if let Err(e) = pusher.push_single_msg(msg).await {
                        error!("推送单聊消息失败: {:?}", e);
                    }
                }
                // 处理群聊消息推送
                MsgType2::Group => {
                    if let Err(e) = pusher.push_group_msg(msg, members).await {
                        error!("推送群聊消息失败: {:?}", e);
                    }
                }
            }
        });
        tasks.push(to_pusher);

        // 等待所有并行任务完成
        // 如果任何任务失败，都会返回错误
        futures::future::try_join_all(tasks)
            .await
            .map_err(|e| Error::Internal(format!("消息处理任务执行失败: {}", e)))?;

        debug!("消息处理完成: server_id={}", msg.server_id);
        Ok(())
    }

    /// 根据消息类型进行分类，确定处理策略
    /// 
    /// 分析消息类型并返回处理策略，包括：
    /// - 消息归类（单聊/群聊）
    /// - 是否需要分配序列号
    /// - 是否需要存储历史记录
    /// 
    /// # 参数
    /// * `msg_type` - 原始消息类型枚举
    /// 
    /// # 返回值
    /// 返回元组: (简化消息类型, 是否需要序列号, 是否需要历史存储)
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
