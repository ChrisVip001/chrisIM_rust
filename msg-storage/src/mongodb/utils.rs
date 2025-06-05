use common::error::Error;
use common::proto::message::Msg;
use bson::{doc, Document};

/// 将消息结构体转换为MongoDB文档
/// 用于将Rust的Msg结构体转换为可以存储到MongoDB的BSON文档格式
pub(crate) fn to_doc(msg: &Msg) -> Result<Document, Error> {
    let document = doc! {
        "local_id": &msg.local_id,
        "server_id": &msg.server_id,
        "create_time": msg.create_time,
        "send_time": msg.send_time,
        "content_type": msg.content_type,
        "content": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: msg.content.clone() },
        "send_id": &msg.send_id,
        "receiver_id": &msg.receiver_id,
        "seq": msg.seq,
        "send_seq": msg.send_seq,
        "msg_type": msg.msg_type,
        "is_read": msg.is_read,
        "group_id": &msg.group_id,
    };

    Ok(document)
}
