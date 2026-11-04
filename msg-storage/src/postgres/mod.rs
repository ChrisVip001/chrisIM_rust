/// PostgreSQL 存储实现
///
/// 统一读扩散：会话序号、消息元数据与正文、用户已读游标。
mod read_spread;

pub(crate) use read_spread::*;
