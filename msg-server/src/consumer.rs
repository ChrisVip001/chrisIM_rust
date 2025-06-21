use std::sync::Arc;

use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::{ClientConfig, Message};
use tracing::{debug, error, info, warn};

use cache::Cache;
use common::config::AppConfig;
use common::error::Error;
use common::proto::message::{GroupMemSeq, Msg, MsgRead, MsgType};
use common::types::msg::MsgType2;
use msg_storage::{msg_rec_box_repo, DbRepo};
use msg_storage::message::MsgRecBoxRepo;
use crate::pusher::{push_service, Pusher};

// 添加群组服务客户端相关导入
use common::grpc_client::base::get_rpc_client;
use common::proto::group::GetMembersRequest;
use common::proto::group::group_service_client::GroupServiceClient;
use common::service_discovery::LbWithServiceDiscovery;

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
/// Kafka队列 -> 消费消息 -> 解析消息 -> 分配序列号 -> 群组成员处理 -> 并行处理:
///                                                      ↓
///                                              查询/更新群组成员缓存
///                                                      ↓
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
    
    /// 群组服务客户端
    /// 用于查询群组成员信息、处理群组相关操作
    group_client: GroupServiceClient<LbWithServiceDiscovery>,
    
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

        // 初始化群组服务客户端
        let group_client = get_rpc_client::<GroupServiceClient<LbWithServiceDiscovery>>(config, "group".to_string()).await?;

        info!("消息消费者服务初始化完成");
        
        Ok(Self {
            consumer,
            db,
            msg_box,
            pusher,
            cache,
            group_client,
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
            // 首先接收消息
            let message = match self.consumer.recv().await {
                Err(e) => {
                    error!("从Kafka接收消息失败: {}", e);
                    // 遇到接收错误时继续循环，不中断服务
                    continue;
                }
                Ok(m) => m,
            };

            // 提取消息负载并转换为拥有所有权的字符串，这样就不再借用message
            let payload_result = message.payload_view::<str>().map(|result| result.map(|s| s.to_owned()));
            
            // 先提交偏移量，释放对message的借用
            if let Err(e) = self.consumer.commit_message(&message, CommitMode::Async) {
                error!("提交Kafka消息偏移量失败: {:?}", e);
            }

            // 现在处理消息内容（不再有借用冲突）
            if let Some(Ok(payload)) = payload_result {
                debug!("收到Kafka消息，长度: {} 字节", payload.len());
                
                // 处理消息内容（这里需要可变借用）
                if let Err(e) = self.handle_msg(&payload).await {
                    error!("处理消息失败: {:?}, 消息内容: {}", e, payload);
                }
            } else {
                warn!("收到空消息或消息解码失败");
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
    async fn handle_msg(&mut self, payload: &str) -> Result<(), Error> {
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
        let (msg_type, need_increase_seq, need_history) = MsgType2::classify_msg_type(mt);
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
                MsgType2::Friend => {
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
                MsgType2::System => {
                    // TODO系统消息暂未实现
                    warn!("系统消息暂未实现: {:?}", msg_type);
                }
            }
        });
        tasks.push(to_pusher);

        // 等待所有并行任务完成
        // 如果任何任务失败，都会返回错误
        futures::future::try_join_all(tasks)
            .await
            .map_err(|e| Error::Internal(format!("消息处理任务执行失败: {}", e)))?;

        Ok(())
    }
    

    /// 从缓存查询群组成员ID列表
    /// 
    /// 首先尝试从Redis缓存中获取群组成员列表，如果缓存中没有数据，
    /// 则从数据库查询并更新缓存。这种策略可以减少数据库访问，提高性能。
    /// 
    /// # 参数
    /// * `group_id` - 群组ID
    /// 
    /// # 返回值
    /// * `Ok(Vec<String>)` - 群组成员ID列表
    /// * `Err(Error)` - 查询失败的错误信息
    async fn get_members_id(&mut self, group_id: &str) -> Result<Vec<String>, Error> {
        match self.cache.query_group_members_id(group_id).await {
            Ok(list) if !list.is_empty() => Ok(list),
            Ok(_) => {
                warn!("缓存中群组成员ID列表为空");
                // 从群组服务查询
                self.query_group_members_id_from_db(group_id).await
            }
            Err(err) => {
                error!("从缓存查询群组成员ID失败: {:?}", err);
                Err(err)
            }
        }
    }

    /// 处理发送者的发送序列号
    /// 
    /// 检查用户的发送序列号是否需要更新到数据库。
    /// 当缓存中的序列号达到步长阈值时，将最新序列号同步到数据库。
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// 
    /// # 返回值
    /// * `Ok(())` - 处理成功
    /// * `Err(Error)` - 处理失败的错误信息
    async fn handle_send_seq(&self, user_id: &str) -> Result<(), Error> {
        let send_seq = self.cache.get_send_seq(user_id).await?;

        // 如果当前序列号已达到步长阈值，需要同步到数据库
        if send_seq.0 == send_seq.1 - self.seq_step as i64 {
            self.db.seq.save_max_seq(user_id).await?;
        }
        Ok(())
    }

    /// 为用户增加消息接收序列号
    /// 
    /// 为指定用户分配一个新的消息接收序列号，用于消息排序和去重。
    /// 如果序列号发生了更新（达到步长阈值），则同步到数据库。
    /// 
    /// # 参数
    /// * `user_id` - 用户ID
    /// 
    /// # 返回值
    /// * `Ok(i64)` - 新分配的序列号
    /// * `Err(Error)` - 分配失败的错误信息
    async fn increase_message_seq(&self, user_id: &str) -> Result<i64, Error> {
        let (cur_seq, _, updated) = self.cache.increase_seq(user_id).await?;
        if updated {
            self.db.seq.save_max_seq(user_id).await?;
        }
        Ok(cur_seq)
    }

    /// 处理消息已读状态更新
    /// 
    /// 解析已读消息的内容，并更新MongoDB中对应消息的已读状态。
    /// 这个操作只影响消息盒子中的离线消息，不影响历史消息。
    /// 
    /// # 参数
    /// * `msg` - 包含已读信息的消息对象
    /// 
    /// # 返回值
    /// * `Ok(())` - 更新成功
    /// * `Err(Error)` - 更新失败的错误信息
    async fn handle_msg_read(&self, msg: Msg) -> Result<(), Error> {
        let data: MsgRead = bincode::deserialize(&msg.content)
            .map_err(|_| Error::Internal("反序列化MsgRead失败".to_string()))?;

        self.msg_box.msg_read(&data.user_id, &data.msg_seq).await?;
        Ok(())
    }

    /// 处理群聊消息的序列号分配
    /// 
    /// 对于群聊消息，需要为每个群成员分配序列号，并处理群组状态变更：
    /// - 群解散：删除缓存中的群成员信息
    /// - 成员退出：从缓存中移除特定成员
    /// - 移除成员：批量移除多个成员
    /// 
    /// # 参数
    /// * `msg_type` - 消息类型分类
    /// * `msg` - 消息对象（可变引用，可能会修改）
    /// 
    /// # 返回值
    /// * `Ok(Vec<GroupMemSeq>)` - 群成员序列号列表
    /// * `Err(Error)` - 处理失败的错误信息
    async fn handle_group_seq(
        &mut self,
        msg_type: &MsgType2,
        msg: &mut Msg,
    ) -> Result<Vec<GroupMemSeq>, Error> {
        if *msg_type != MsgType2::Group {
            return Ok(vec![]);
        }
        // 从缓存查询群组成员ID
        let mut members = self.get_members_id(&msg.receiver_id).await?;

        // 排除发送者自己（发送者不需要接收自己的消息）
        members.retain(|id| id != &msg.send_id);

        // 为所有群成员增加序列号
        let seq = self.cache.incr_group_seq(members).await?;

        // 根据消息类型判断是否需要更新缓存
        // 如果是群解散，应该删除缓存数据
        // 如果是成员退出，应该更新缓存
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

    /// 判断消息是否需要存储到数据库
    /// 
    /// 某些消息类型（如音视频通话协议相关的消息）不需要持久化存储，
    /// 因为它们只是实时通信的控制信息，没有保存价值。
    /// 
    /// # 参数
    /// * `msg_type` - 消息类型
    /// 
    /// # 返回值
    /// * `true` - 需要存储到数据库
    /// * `false` - 不需要存储到数据库
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

    /// 将消息存储到数据库
    /// 
    /// 根据消息类型选择不同的存储策略：
    /// - 单聊消息：存储到PostgreSQL历史表和MongoDB消息盒子
    /// - 群聊消息：为每个群成员创建消息副本并存储
    /// 
    /// # 参数
    /// * `db` - 数据库仓库实例
    /// * `msg_box` - MongoDB消息盒子仓库
    /// * `msg` - 要存储的消息
    /// * `msg_type` - 消息类型分类
    /// * `need_to_history` - 是否需要存储历史记录
    /// * `members` - 群成员序列号列表（仅群聊消息使用）
    /// 
    /// # 返回值
    /// * `Ok(())` - 存储成功
    /// * `Err(Error)` - 存储失败的错误信息
    async fn send_to_db(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        msg: Msg,
        msg_type: MsgType2,
        need_to_history: bool,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        // 根据消息类型选择不同的处理方法
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

    /// 从群组服务查询群组成员ID
    /// 并将结果设置到缓存中
    /// 
    /// 当缓存中没有群组成员信息时，从群组服务查询并更新缓存。
    /// 
    /// # 参数
    /// * `group_id` - 群组ID
    /// 
    /// # 返回值
    /// * `Ok(Vec<String>)` - 群组成员ID列表
    /// * `Err(Error)` - 查询失败的错误信息
    async fn query_group_members_id_from_db(&mut self, group_id: &str) -> Result<Vec<String>, Error> {
        info!("从群组服务查询群组成员ID: {}", group_id);

        // 调用群组服务获取成员列表
        let request = GetMembersRequest {
            group_id: group_id.to_string(),
            page: 1,
            page_size: 5000,
        };
        match self.group_client.get_members(request).await {
            Ok(response) => {
                // 提取成员ID列表
                let members_id: Vec<String> = response.into_inner().members
                    .into_iter()
                    .map(|member| member.user_id)
                    .collect();
                
                info!("从群组服务查询到 {} 个成员ID", members_id.len());
                
                // 将查询结果保存到缓存
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
                error!("从群组服务查询成员ID失败: {:?}", e);
                // 查询失败时返回空列表，避免中断消息处理流程
                Ok(Vec::new())
            }
        }
    }

    /// 处理单聊消息的存储
    /// 
    /// 并行执行两个存储任务：
    /// 1. 如果需要历史记录，存储到PostgreSQL
    /// 2. 存储到MongoDB消息盒子（用于离线消息）
    /// 
    /// 对于某些特殊消息类型（如群组操作确认、好友关系确认），
    /// 会从MongoDB中删除而不是保存。
    /// 
    /// # 参数
    /// * `db` - 数据库仓库实例
    /// * `msg_box` - MongoDB消息盒子仓库
    /// * `message` - 要处理的消息
    /// * `need_to_history` - 是否需要存储历史记录
    /// 
    /// # 返回值
    /// * `Ok(())` - 处理成功
    /// * `Err(Error)` - 处理失败的错误信息
    async fn handle_message(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        message: Msg,
        need_to_history: bool,
    ) -> Result<(), Error> {
        // 任务1：保存消息到PostgreSQL

        let mut tasks = Vec::with_capacity(2);
        if need_to_history {
            let cloned_msg = message.clone();
            let db_task = tokio::spawn(async move {
                if let Err(e) = db.msg.save_message(cloned_msg).await {
                    tracing::error!("保存消息到数据库失败: {}", e);
                }
            });
            tasks.push(db_task);
        }

        // 任务2：保存消息到MongoDB
        let msg_rec_box_task = tokio::spawn(async move {
            // 如果消息类型是群组操作确认 好友请求 和好友请求确认，我们应该从MongoDB中删除它 
            if message.msg_type == MsgType::GroupDismissOrExitReceived as i32
                || message.msg_type == MsgType::GroupInvitationReceived as i32
                || message.msg_type == MsgType::FriendApplyReq as i32
            {
                if let Err(e) = msg_box.delete_message(&message.server_id).await {
                    tracing::error!("从MongoDB删除消息失败: {}", e);
                }
                return;
            }
            if let Err(e) = msg_box.save_message(&message).await {
                tracing::error!("保存消息到MongoDB失败: {}", e);
            }
        });
        tasks.push(msg_rec_box_task);

        // 等待所有任务完成
        futures::future::try_join_all(tasks)
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(())
    }

    /// 处理群聊消息的存储
    /// 
    /// 并行执行两个主要任务：
    /// 1. 更新PostgreSQL中的用户序列号，如果需要则保存消息历史
    /// 2. 为每个群成员在MongoDB中创建消息副本
    /// 
    /// # 参数
    /// * `db` - 数据库仓库实例
    /// * `msg_box` - MongoDB消息盒子仓库
    /// * `message` - 要处理的群聊消息
    /// * `need_to_history` - 是否需要存储历史记录
    /// * `members` - 群成员序列号列表
    /// 
    /// # 返回值
    /// * `Ok(())` - 处理成功
    /// * `Err(Error)` - 处理失败的错误信息
    async fn handle_group_message(
        db: Arc<DbRepo>,
        msg_box: Arc<dyn MsgRecBoxRepo>,
        message: Msg,
        need_to_history: bool,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        // 任务1：保存消息到PostgreSQL
        // 更新用户在PostgreSQL中的序列号
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
                    tracing::error!("批量保存最大序列号失败: {}", err);
                    return Err(err);
                };
            }

            if let Some(cloned_msg) = cloned_msg {
                if let Err(e) = db.msg.save_message(cloned_msg).await {
                    tracing::error!("保存消息到数据库失败: {}", e);
                    return Err(e);
                }
            }
            Ok(())
        });

        // 任务2：保存消息到MongoDB
        let msg_rec_box_task = tokio::spawn(async move {
            if let Err(e) = msg_box.save_group_msg(message, members).await {
                tracing::error!("保存消息到MongoDB失败: {}", e);
                return Err(e);
            }
            Ok(())
        });

        // 等待所有任务完成
        let (db_result, msg_rec_box_result) = tokio::try_join!(db_task, msg_rec_box_task)
            .map_err(|e| Error::Internal(e.to_string()))?;

        db_result?;
        msg_rec_box_result?;

        Ok(())
    }
}
