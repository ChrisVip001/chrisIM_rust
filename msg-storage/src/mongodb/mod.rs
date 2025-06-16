/// MongoDB数据库相关实现模块
/// 包含消息接收箱和消息清理功能的MongoDB实现

mod message;
mod utils;
mod test;

pub(crate) use message::*;
