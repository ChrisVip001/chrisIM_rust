use std::fmt::Debug;

use async_trait::async_trait;
use futures::stream::TryStreamExt;
use mongodb::options::{FindOptions, IndexOptions};
use mongodb::{
    bson::{doc, Document},
    Client, Collection, Database, IndexModel,
};
use tokio::sync::mpsc;
use tonic::codegen::tokio_stream::StreamExt;
use tracing::log::{error};

use common::config::AppConfig;
use common::error::Error;
use common::proto::message::{GroupMemSeq, Msg, MsgType};

use crate::message::{MsgRecBoxCleaner, MsgRecBoxRepo};
use crate::mongodb::utils::to_doc;

/// 用户消息接收箱
/// 需要对消息进行分类管理
/// 如：群组消息、单聊消息、系统消息、服务消息、第三方消息等
/// 或者为每个用户设置一个集合
#[derive(Debug)]
pub struct MsgBox {
    /// 消息接收箱集合
    mb: Collection<Document>,
}

/// 所有用户单聊消息接收箱集合名称
const COLL_SINGLE_BOX: &str = "single_msg_box";

#[allow(dead_code)]
impl MsgBox {
    pub async fn new(db: Database) -> Self {
        let mb = db.collection(COLL_SINGLE_BOX);
        Self { mb }
    }
    
    /// 从配置创建消息接收箱实例
    /// 自动创建必要的索引以优化查询性能
    pub async fn from_config(config: &AppConfig) -> Result<Self, Error> {
        let client = Client::with_uri_str(config.database.mongo_url())
            .await
            .map_err(|e| Error::Internal(format!("MongoDB 连接失败: {}", e)))?;
        
        let db = client.database(&config.database.mongodb.database);
        let mb = db.collection(COLL_SINGLE_BOX);

        // 创建 receiver_id 和 seq 的复合索引
        let index_model = IndexModel::builder()
            .keys(doc! {"receiver_id": 1, "seq":1})
            .options(IndexOptions::builder().unique(false).build())
            .build();
        
        if let Err(e) = mb.create_index(index_model).await {
            error!("创建索引失败: {}", e);
        }

        // 创建 send_id 和 send_seq 的复合索引
        let index_model = IndexModel::builder()
            .keys(doc! {"send_id": 1, "send_seq":1})
            .options(IndexOptions::builder().unique(false).build())
            .build();
        
        if let Err(e) = mb.create_index(index_model).await {
            error!("创建复合索引失败: {}", e);
        }

        // 创建复合索引
        let index_model = IndexModel::builder()
            .keys(doc! {
                "group_id": 1,
                "send_time": -1
            })
            .build();

        if let Err(e) = mb.create_index(index_model).await {
            error!("创建复合索引失败: {}", e);
        }

        Ok(Self { mb })
    }
}

#[async_trait]
impl MsgRecBoxRepo for MsgBox {
    /// 保存单条消息
    async fn save_message(&self, message: &Msg) -> Result<(), Error> {
        self.mb.insert_one(to_doc(message)?).await?;
        Ok(())
    }

    /// 保存群组消息到接收箱
    /// 为发送者和所有群组成员分别保存消息副本
    async fn save_group_msg(
        &self,
        mut message: Msg,
        members: Vec<GroupMemSeq>,
    ) -> Result<(), Error> {
        let mut messages = Vec::with_capacity(members.len() + 1);
        
        // 为发送者保存消息（保持原始send_seq，因为这是他发送的消息）
        messages.push(to_doc(&message)?);

        // 为每个群组成员保存消息副本（接收者的send_seq应该是0）
        let original_send_seq = message.send_seq;
        message.send_seq = 0; // 对于接收者，send_seq应该是0
        
        for seq in members {
            // 设置成员的接收序列号
            message.seq = seq.cur_seq;
            message.receiver_id = seq.mem_id;
            messages.push(to_doc(&message)?);
        }
        
        self.mb.insert_many(messages).await?;
        Ok(())
    }

    /// 删除单条消息
    async fn delete_message(&self, message_id: &str) -> Result<(), Error> {
        let query = doc! {"server_id": message_id};
        self.mb.delete_one(query).await?;
        Ok(())
    }

    /// 根据用户ID和消息序列号批量删除消息
    async fn delete_messages(&self, user_id: &str, msg_seq: Vec<i64>) -> Result<(), Error> {
        let query = doc! {"receiver_id": user_id, "seq": {"$in": msg_seq}};
        self.mb.delete_many(query).await?;
        Ok(())
    }

    /// 撤回消息
    /// 将指定消息标记为已撤回状态，并设置撤回时间和撤回者
    async fn revoke_message(&self, message_id: &str, user_id: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().timestamp_millis();
        
        // 查询条件：消息ID匹配且发送者是当前用户
        let query = doc! {
            "server_id": message_id,
            "send_id": user_id
        };
        
        // 更新字段：标记为已撤回，设置撤回时间和撤回者
        let update = doc! {
            "$set": {
                "is_revoked": true,
                "revoke_time": now,
                "revoked_by": user_id
            }
        };
        
        let result = self.mb.update_many(query, update).await?;
        
        // 检查是否有消息被更新
        if result.modified_count == 0 {
            return Err(Error::NotFound("消息不存在或无权撤回".to_string()));
        }
        
        Ok(())
    }

    /// 根据消息ID获取单条消息
    async fn get_message(&self, message_id: &str) -> Result<Option<Msg>, Error> {
        let result = self
            .mb
            .find_one(doc! {"server_id": message_id})
            .await?;

        match result {
            Some(doc) => Ok(Some(Msg::try_from(doc)?)),
            None => Ok(None),
        }
    }

    /// 获取用户消息流
    /// 使用流式处理，适合处理大量消息数据
    async fn get_messages_stream(
        &self,
        user_id: &str,
        start: i64,
        end: i64,
    ) -> Result<mpsc::Receiver<Result<Msg, Error>>, Error> {
        let query = doc! {
            "receiver_id": user_id,
            "seq": {
                "$gte": start,
                "$lte": end
            }
        };

        // 按序列号排序
        let option = FindOptions::builder().sort(doc! {"seq": 1}).build();

        // 执行查询
        let mut cursor = self.mb.find(query).with_options(option).await?;

        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(result) = cursor.next().await {
                match result {
                    Ok(doc) => {
                        match Msg::try_from(doc) {
                            Ok(msg) => {
                                if tx.send(Ok(msg)).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                if tx.send(Err(e.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if tx.send(Err(e.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    /// 获取用户消息列表（已废弃）
    /// 建议使用 get_messages_stream 方法
    async fn get_messages(&self, user_id: &str, start: i64, end: i64) -> Result<Vec<Msg>, Error> {
        let query = doc! {
            "receiver_id": user_id,
            "seq": {
                "$gte": start,
                "$lte": end
            }
        };

        // 按序列号排序
        let option = FindOptions::builder().sort(doc! {"seq": 1}).build();

        // 执行查询
        let mut cursor = self.mb.find(query).with_options(option).await?;
        let mut messages = Vec::with_capacity((end - start) as usize);
        while let Some(result) = cursor.next().await {
            let msg = Msg::try_from(result?)?;
            messages.push(msg)
        }

        Ok(messages)
    }

    /// 获取用户的发送和接收消息
    /// 支持分别指定发送消息和接收消息的序列号范围
    /// 返回按时间和序列号排序的消息列表
    async fn get_msgs(
        &self,
        user_id: &str,
        send_start: i64,
        send_end: i64,
        rec_start: i64,
        rec_end: i64,
    ) -> Result<Vec<Msg>, Error> {
        let pipeline = vec![
            doc! {
                "$match": {
                    "$or": [
                        {
                            "receiver_id": user_id,
                            "seq": { "$gte": rec_start, "$lte": rec_end }
                        },
                        {
                            "send_id": user_id,
                            "send_seq": { "$gte": send_start, "$lte": send_end }
                        }
                    ]
                }
            },
            doc! {
                "$addFields": {
                    "sort_field": {
                        "$cond": {
                            "if": { "$eq": ["$send_id", user_id] },
                            "then": "$send_seq",
                            "else": "$seq"
                        }
                    }
                }
            },
            doc! {
                "$sort": {
                    "send_time": 1,
                    "sort_field": 1
                }
            },
        ];

        // 防止整数溢出和内存分配问题
        // 计算预期的消息数量，但要防止溢出
        let send_range = send_end.saturating_sub(send_start).max(0);
        let rec_range = rec_end.saturating_sub(rec_start).max(0);
        let estimated_len = send_range.saturating_add(rec_range);
        
        // 限制最大容量，防止内存溢出
        const MAX_CAPACITY: i64 = 100_000; // 最多预分配10万条消息的空间
        let safe_capacity = estimated_len.min(MAX_CAPACITY);
        
        // 执行聚合查询
        let mut cursor = self.mb.aggregate(pipeline).await?;

        let mut messages = Vec::with_capacity(safe_capacity as usize);
        while let Some(result) = cursor.next().await {
            let mut msg = Msg::try_from(result?)?;
            // 如果消息是用户发送的，将seq设置为0
            if user_id == msg.send_id {
                msg.seq = 0;
            }
            messages.push(msg)
        }
        Ok(messages)
    }

    /// 标记消息为已读
    /// 根据用户ID和消息序列号批量更新消息的已读状态
    async fn msg_read(&self, user_id: &str, msg_seq: &[i64]) -> Result<(), Error> {
        if msg_seq.is_empty() {
            return Ok(());
        }
        let query = doc! {"receiver_id":{"$eq":user_id},"seq":{"$in":msg_seq}};
        let update = doc! {"$set":{"is_read":true}};
        self.mb.update_many(query, update).await?;
        Ok(())
    }

    /// 根据会话ID标记所有消息为已读
    async fn mark_conversation_read(&self, user_id: &str, conversation_id: &str, up_to_time: Option<i64>) -> Result<i32, Error> {
        // 构建查询条件
        let mut query = doc! {
            "receiver_id": user_id,
            "is_read": false, // 只标记未读消息
            "is_revoked": false, // 不标记已撤回的消息
        };

        // 根据消息类型确定会话匹配条件
        // 对于群聊消息，group_id 等于 conversation_id
        // 对于单聊消息，send_id 等于 conversation_id（对方的用户ID）
        query.insert("$or", vec![
            doc! {
                "group_id": conversation_id,
                "msg_type": MsgType::GroupMsg as i32
            },
            doc! {
                "send_id": conversation_id,
                "msg_type": MsgType::SingleMsg as i32
            }
        ]);

        // 如果指定了时间范围，只标记该时间之前的消息
        if let Some(time) = up_to_time {
            query.insert("send_time", doc! {"$lte": time});
        }

        // 执行更新操作
        let update = doc! {"$set": {"is_read": true}};
        let result = self.mb.update_many(query, update).await?;
        
        Ok(result.modified_count as i32)
    }

    /// 根据消息ID删除消息（支持批量）
    async fn delete_messages_by_ids(&self, user_id: &str, message_ids: &[String]) -> Result<i32, Error> {
        if message_ids.is_empty() {
            return Ok(0);
        }
        
        // 查询条件：消息ID在列表中且接收者是当前用户
        let query = doc! {
            "server_id": {"$in": message_ids},
            "receiver_id": user_id
        };
        
        let result = self.mb.delete_many(query).await?;
        Ok(result.deleted_count as i32)
    }

    /// 根据会话ID和序列号范围获取消息历史
    async fn get_conversation_messages_by_seq_range(
        &self,
        user_id: &str,
        conversation_id: &str,
        send_seq_start: i64,
        send_seq_end: i64,
        seq_start: i64,
        seq_end: i64,
    ) -> Result<Vec<Msg>, Error> {
        // 构建查询条件
        let mut query = doc! {
            "$or": [
                {
                    // 用户接收的消息，且属于指定会话
                    "receiver_id": user_id,
                    "$or": [
                        {
                            // 群聊消息：group_id 等于 conversation_id
                            "group_id": conversation_id,
                            "msg_type": MsgType::GroupMsg as i32
                        },
                        {
                            // 单聊消息：send_id 等于 conversation_id（对方发给我的）
                            "send_id": conversation_id,
                            "msg_type": MsgType::SingleMsg as i32
                        }
                    ]
                },
                {
                    // 用户发送的消息，且属于指定会话
                    "send_id": user_id,
                    "$or": [
                        {
                            // 群聊消息：group_id 等于 conversation_id
                            "group_id": conversation_id,
                            "msg_type": MsgType::GroupMsg as i32
                        },
                        {
                            // 单聊消息：receiver_id 等于 conversation_id（我发给对方的）
                            "receiver_id": conversation_id,
                            "msg_type": MsgType::SingleMsg as i32
                        }
                    ]
                }
            ]
        };

        // 添加序列号范围过滤条件
        // 使用复杂的逻辑表达式来处理序列号范围
        let mut seq_conditions = vec![];

        // 如果指定了接收消息的序列号范围
        if seq_start > 0 || seq_end > 0 {
            let mut recv_condition = doc! {
                "receiver_id": user_id
            };
            
            if seq_start > 0 && seq_end > 0 {
                recv_condition.insert("seq", doc! {"$gte": seq_start, "$lte": seq_end});
            } else if seq_start > 0 {
                recv_condition.insert("seq", doc! {"$gte": seq_start});
            } else if seq_end > 0 {
                recv_condition.insert("seq", doc! {"$lte": seq_end});
            }
            
            seq_conditions.push(recv_condition);
        }

        // 如果指定了发送消息的序列号范围
        if send_seq_start > 0 || send_seq_end > 0 {
            let mut send_condition = doc! {
                "send_id": user_id
            };
            
            if send_seq_start > 0 && send_seq_end > 0 {
                send_condition.insert("send_seq", doc! {"$gte": send_seq_start, "$lte": send_seq_end});
            } else if send_seq_start > 0 {
                send_condition.insert("send_seq", doc! {"$gte": send_seq_start});
            } else if send_seq_end > 0 {
                send_condition.insert("send_seq", doc! {"$lte": send_seq_end});
            }
            
            seq_conditions.push(send_condition);
        }

        // 如果有序列号条件，与会话过滤条件组合
        if !seq_conditions.is_empty() {
            query = doc! {
                "$and": [
                    query,
                    {
                        "$or": seq_conditions
                    }
                ]
            };
        }

        // 按时间正序排列
        let options = FindOptions::builder()
            .sort(doc! {"send_time": 1})
            .build();

        // 执行查询
        let mut cursor = self.mb.find(query).with_options(options).await?;
        let mut messages = Vec::new();
        
        while let Some(result) = cursor.next().await {
            let mut msg = Msg::try_from(result?)?;
            
            // 如果消息是用户发送的，将接收序列号设置为0
            if user_id == msg.send_id {
                msg.seq = 0;
            }
            
            messages.push(msg);
        }

        Ok(messages)
    }
}

impl MsgRecBoxCleaner for MsgBox {
    /// 启动消息接收箱清理任务
    /// 定期删除过期消息，保留指定类型的消息不被清理
    fn clean_receive_box(&self, period: i64, types: Vec<i32>) {
        let mb = self.mb.clone();

        tokio::spawn(async move {
            let retention_duration = chrono::Duration::days(period);
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(24 * 60 * 60)); // 每24小时执行一次
            loop {
                interval.tick().await;
                let now = chrono::Utc::now();
                let cutoff_time = (now - retention_duration).timestamp();

                let result = mb
                    .delete_many(
                        doc! {
                            "send_time": { "$lt": cutoff_time },
                            "msg_type": { "$nin": types.clone()}
                        },
                    )
                    .await;

                match result {
                    Ok(delete_result) => {
                        println!("已删除 {} 条过期消息", delete_result.deleted_count);
                    }
                    Err(e) => {
                        eprintln!("删除过期消息时发生错误: {:?}", e);
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use common::proto::message::{MsgType, PlatformType};
    use super::super::test::MongoDbTester;

    use super::*;

    struct TestConfig {
        box_: MsgBox,
        _tdb: MongoDbTester,
    }

    impl Deref for TestConfig {
        type Target = MsgBox;
        fn deref(&self) -> &Self::Target {
            &self.box_
        }
    }

    impl TestConfig {
        pub async fn new() -> Self {
            let config = AppConfig::from_file(Option::from("./config/config.yml")).expect("config error");
            let tdb = MongoDbTester::new(
                &config.database.mongodb.host,
                config.database.mongodb.port,
                config.database.mongodb.user.as_deref().unwrap_or(""),
                config.database.mongodb.password.as_deref().unwrap_or(""),
            )
            .await;
            let msg_box = MsgBox::new(tdb.database().await).await;
            Self {
                box_: msg_box,
                _tdb: tdb,
            }
        }
    }
    #[tokio::test]
    async fn mongodb_insert_and_get_works() {
        let msg_box = TestConfig::new().await;
        let msg_id = "123";
        let msg = get_test_msg(msg_id.to_string());
        // save it into mongodb
        msg_box.save_message(&msg).await.unwrap();
        let msg = msg_box.get_message(msg_id).await.unwrap();
        assert!(msg.is_some());
        assert_eq!(msg.unwrap().server_id, msg_id);
    }

    #[tokio::test]
    async fn mongodb_insert_and_delete_and_get_works() {
        let msg_box = TestConfig::new().await;
        let msg_id = "123";
        let msg = get_test_msg(msg_id.to_string());
        // save it into mongodb
        msg_box.save_message(&msg).await.unwrap();

        // delete it
        msg_box.delete_message(msg_id).await.unwrap();

        let msg = msg_box.get_message(msg_id).await.unwrap();
        assert!(msg.is_none());
    }

    fn get_test_msg(msg_id: String) -> Msg {
        Msg {
            local_id: "123".to_string(),
            server_id: msg_id,
            create_time: 0,
            send_time: chrono::Local::now().timestamp(),
            content_type: 0,
            content: "test".to_string().into_bytes(),
            send_id: "123".to_string(),
            receiver_id: "111".to_string(),
            seq: 0,
            send_seq: 0,
            msg_type: MsgType::SingleMsg as i32,
            is_read: false,
            platform: PlatformType::Mobile as i32,
            group_id: "".to_string(),
            avatar: "".to_string(),
            nickname: "".to_string(),
            related_msg_id: None,
            is_revoked: false,
            revoke_time: 0,
            revoked_by: String::new(),
            forward_comment: None,
            is_forwarded: false,
            is_reply: false,
        }
    }
    #[tokio::test]
    async fn mongodb_insert_and_batch_delete_and_get_should_works() {
        let msg_box = TestConfig::new().await;
        let msg_id = vec!["123".to_string(), "124".to_string(), "125".to_string()];
        let mut msg = get_test_msg(msg_id[0].clone());

        let msg_seq = vec![12, 123, 1234];
        // save it into mongodb
        msg_box.save_message(&msg).await.unwrap();

        msg.server_id.clone_from(&msg_id[1]);
        msg.seq = msg_seq[1];
        msg_box.save_message(&msg).await.unwrap();

        msg.seq = msg_seq[2];
        msg.server_id.clone_from(&msg_id[2]);
        msg_box.save_message(&msg).await.unwrap();

        // delete it
        msg_box
            .delete_messages("111", msg_seq.clone())
            .await
            .unwrap();

        let msg = msg_box.get_message(&msg_id[0]).await.unwrap();
        assert!(msg.is_none());

        let msg = msg_box.get_message(&msg_id[1]).await.unwrap();
        assert!(msg.is_none());

        let msg = msg_box.get_message(&msg_id[2]).await.unwrap();
        assert!(msg.is_none());
    }
}
