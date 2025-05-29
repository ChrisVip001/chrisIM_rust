/// 消息扩展模块
/// 
/// 为自动生成的Msg结构体添加语义清晰的辅助方法，
/// 提高代码可读性和维护性。

use crate::message::{Msg, MsgType, ContentType};

/// 消息扩展trait
/// 
/// 为Msg结构体提供语义清晰的辅助方法，
/// 帮助开发者更好地理解和使用消息对象。
pub trait MsgExt {
    /// 判断是否为群聊消息
    /// 
    /// # 返回值
    /// * `true` - 群聊消息
    /// * `false` - 单聊消息
    fn is_group_message(&self) -> bool;
    
    /// 判断是否为单聊消息
    /// 
    /// # 返回值
    /// * `true` - 单聊消息
    /// * `false` - 群聊消息
    fn is_private_message(&self) -> bool;
    
    /// 获取会话ID
    /// 
    /// 对于单聊消息，返回对方用户ID；
    /// 对于群聊消息，返回群组ID。
    /// 
    /// # 返回值
    /// 会话的唯一标识符
    fn get_conversation_id(&self) -> &str;
    
    /// 获取消息类型的字符串表示
    /// 
    /// # 返回值
    /// 消息类型的可读字符串
    fn get_msg_type_name(&self) -> &'static str;
    
    /// 获取内容类型的字符串表示
    /// 
    /// # 返回值
    /// 内容类型的可读字符串
    fn get_content_type_name(&self) -> &'static str;
    
    /// 判断是否为文本消息
    /// 
    /// # 返回值
    /// * `true` - 文本消息
    /// * `false` - 其他类型消息
    fn is_text_message(&self) -> bool;
    
    /// 判断是否为媒体消息（图片、视频、音频）
    /// 
    /// # 返回值
    /// * `true` - 媒体消息
    /// * `false` - 其他类型消息
    fn is_media_message(&self) -> bool;
    
    /// 判断是否为系统消息
    /// 
    /// # 返回值
    /// * `true` - 系统消息（群组操作、好友操作等）
    /// * `false` - 用户消息
    fn is_system_message(&self) -> bool;
    
    /// 判断是否为通话相关消息
    /// 
    /// # 返回值
    /// * `true` - 通话消息
    /// * `false` - 其他类型消息
    fn is_call_message(&self) -> bool;
    
    /// 判断是否需要存储到历史记录
    /// 
    /// 某些临时消息（如通话协商信息）不需要持久化存储
    /// 
    /// # 返回值
    /// * `true` - 需要存储
    /// * `false` - 不需要存储
    fn should_store_to_history(&self) -> bool;
    
    /// 判断是否需要推送通知
    /// 
    /// # 返回值
    /// * `true` - 需要推送
    /// * `false` - 不需要推送
    fn should_push_notification(&self) -> bool;
    
    /// 获取消息摘要
    /// 
    /// 用于显示在会话列表中的消息预览
    /// 
    /// # 返回值
    /// 消息的简短摘要
    fn get_summary(&self) -> String;
    
    /// 验证消息字段的有效性
    /// 
    /// # 返回值
    /// * `Ok(())` - 验证通过
    /// * `Err(String)` - 验证失败，包含错误信息
    fn validate(&self) -> Result<(), String>;
}

impl MsgExt for Msg {
    fn is_group_message(&self) -> bool {
        !self.group_id.is_empty()
    }
    
    fn is_private_message(&self) -> bool {
        !self.is_group_message()
    }
    
    fn get_conversation_id(&self) -> &str {
        if self.is_group_message() {
            &self.group_id
        } else {
            &self.receiver_id
        }
    }
    
    fn get_msg_type_name(&self) -> &'static str {
        match MsgType::try_from(self.msg_type) {
            Ok(MsgType::SingleMsg) => "单聊消息",
            Ok(MsgType::GroupMsg) => "群聊消息",
            Ok(MsgType::GroupInvitation) => "群组邀请",
            Ok(MsgType::GroupInviteNew) => "邀请新成员",
            Ok(MsgType::GroupMemberExit) => "成员退出群组",
            Ok(MsgType::GroupRemoveMember) => "移除群成员",
            Ok(MsgType::GroupDismiss) => "解散群组",
            Ok(MsgType::GroupDismissOrExitReceived) => "群组解散确认",
            Ok(MsgType::GroupInvitationReceived) => "群组邀请确认",
            Ok(MsgType::GroupUpdate) => "群组信息更新",
            Ok(MsgType::FriendApplyReq) => "好友申请",
            Ok(MsgType::FriendApplyResp) => "好友申请响应",
            Ok(MsgType::FriendBlack) => "拉黑好友",
            Ok(MsgType::FriendDelete) => "删除好友",
            Ok(MsgType::SingleCallInvite) => "通话邀请",
            Ok(MsgType::RejectSingleCall) => "拒绝通话",
            Ok(MsgType::AgreeSingleCall) => "同意通话",
            Ok(MsgType::SingleCallInviteNotAnswer) => "通话无应答",
            Ok(MsgType::SingleCallInviteCancel) => "取消通话",
            Ok(MsgType::SingleCallOffer) => "通话Offer",
            Ok(MsgType::Hangup) => "挂断通话",
            Ok(MsgType::ConnectSingleCall) => "连接通话",
            Ok(MsgType::Candidate) => "WebRTC候选者",
            Ok(MsgType::Read) => "消息已读",
            Ok(MsgType::MsgRecResp) => "消息接收响应",
            Ok(MsgType::Notification) => "系统通知",
            Ok(MsgType::Service) => "服务消息",
            Ok(MsgType::FriendshipReceived) => "好友关系确认",
            Err(_) => "未知消息类型",
        }
    }
    
    fn get_content_type_name(&self) -> &'static str {
        match ContentType::try_from(self.content_type) {
            Ok(ContentType::Default) => "默认",
            Ok(ContentType::Text) => "文本",
            Ok(ContentType::Image) => "图片",
            Ok(ContentType::Video) => "视频",
            Ok(ContentType::Audio) => "音频",
            Ok(ContentType::File) => "文件",
            Ok(ContentType::Emoji) => "表情",
            Ok(ContentType::VideoCall) => "视频通话",
            Ok(ContentType::AudioCall) => "音频通话",
            Ok(ContentType::Error) => "错误",
            Err(_) => "未知内容类型",
        }
    }
    
    fn is_text_message(&self) -> bool {
        matches!(
            ContentType::try_from(self.content_type),
            Ok(ContentType::Text)
        )
    }
    
    fn is_media_message(&self) -> bool {
        matches!(
            ContentType::try_from(self.content_type),
            Ok(ContentType::Image | ContentType::Video | ContentType::Audio | ContentType::File)
        )
    }
    
    fn is_system_message(&self) -> bool {
        matches!(
            MsgType::try_from(self.msg_type),
            Ok(MsgType::GroupInvitation
                | MsgType::GroupInviteNew
                | MsgType::GroupMemberExit
                | MsgType::GroupRemoveMember
                | MsgType::GroupDismiss
                | MsgType::GroupDismissOrExitReceived
                | MsgType::GroupInvitationReceived
                | MsgType::GroupUpdate
                | MsgType::FriendApplyReq
                | MsgType::FriendApplyResp
                | MsgType::FriendBlack
                | MsgType::FriendDelete
                | MsgType::Notification
                | MsgType::Service
                | MsgType::FriendshipReceived)
        )
    }
    
    fn is_call_message(&self) -> bool {
        matches!(
            MsgType::try_from(self.msg_type),
            Ok(MsgType::SingleCallInvite
                | MsgType::RejectSingleCall
                | MsgType::AgreeSingleCall
                | MsgType::SingleCallInviteNotAnswer
                | MsgType::SingleCallInviteCancel
                | MsgType::SingleCallOffer
                | MsgType::Hangup
                | MsgType::ConnectSingleCall
                | MsgType::Candidate)
        )
    }
    
    fn should_store_to_history(&self) -> bool {
        // 通话协商相关的消息不需要存储到历史记录
        !matches!(
            MsgType::try_from(self.msg_type),
            Ok(MsgType::ConnectSingleCall
                | MsgType::AgreeSingleCall
                | MsgType::Candidate
                | MsgType::SingleCallOffer
                | MsgType::SingleCallInvite)
        )
    }
    
    fn should_push_notification(&self) -> bool {
        // 大部分消息都需要推送，除了一些内部协议消息
        !matches!(
            MsgType::try_from(self.msg_type),
            Ok(MsgType::Candidate
                | MsgType::SingleCallOffer
                | MsgType::ConnectSingleCall
                | MsgType::Read
                | MsgType::MsgRecResp)
        )
    }
    
    fn get_summary(&self) -> String {
        if self.is_text_message() {
            // 对于文本消息，显示内容摘要
            let content = String::from_utf8_lossy(&self.content);
            if content.len() > 50 {
                format!("{}...", &content[..50])
            } else {
                content.to_string()
            }
        } else if self.is_media_message() {
            // 对于媒体消息，显示类型
            format!("[{}]", self.get_content_type_name())
        } else if self.is_system_message() {
            // 对于系统消息，显示操作类型
            self.get_msg_type_name().to_string()
        } else if self.is_call_message() {
            // 对于通话消息，显示通话状态
            format!("[{}]", self.get_msg_type_name())
        } else {
            // 其他消息类型
            format!("[{}]", self.get_msg_type_name())
        }
    }
    
    fn validate(&self) -> Result<(), String> {
        // 验证必填字段
        if self.send_id.is_empty() {
            return Err("发送者ID不能为空".to_string());
        }
        
        if self.receiver_id.is_empty() {
            return Err("接收者ID不能为空".to_string());
        }
        
        if self.local_id.is_empty() {
            return Err("本地消息ID不能为空".to_string());
        }
        
        // 验证时间戳
        if self.send_time <= 0 {
            return Err("发送时间必须大于0".to_string());
        }
        
        // 验证群聊消息的群组ID
        if self.is_group_message() && self.group_id.is_empty() {
            return Err("群聊消息的群组ID不能为空".to_string());
        }
        
        // 验证单聊消息不应该有群组ID
        if self.is_private_message() && !self.group_id.is_empty() {
            return Err("单聊消息不应该包含群组ID".to_string());
        }
        
        // 验证序列号
        if self.seq < 0 {
            return Err("接收序列号不能为负数".to_string());
        }
        
        if self.send_seq < 0 {
            return Err("发送序列号不能为负数".to_string());
        }
        
        Ok(())
    }
}

/// 消息构建器
/// 
/// 提供链式调用的方式来构建消息对象，
/// 确保消息字段的正确性和完整性。
#[derive(Debug, Default)]
pub struct MsgBuilder {
    msg: Msg,
}

impl MsgBuilder {
    /// 创建新的消息构建器
    pub fn new() -> Self {
        Self {
            msg: Msg::default(),
        }
    }
    
    /// 设置发送者ID
    pub fn send_id(mut self, send_id: impl Into<String>) -> Self {
        self.msg.send_id = send_id.into();
        self
    }
    
    /// 设置接收者ID
    pub fn receiver_id(mut self, receiver_id: impl Into<String>) -> Self {
        self.msg.receiver_id = receiver_id.into();
        self
    }
    
    /// 设置群组ID（用于群聊消息）
    pub fn group_id(mut self, group_id: impl Into<String>) -> Self {
        self.msg.group_id = group_id.into();
        self
    }
    
    /// 设置本地消息ID
    pub fn local_id(mut self, local_id: impl Into<String>) -> Self {
        self.msg.local_id = local_id.into();
        self
    }
    
    /// 设置服务器消息ID
    pub fn server_id(mut self, server_id: impl Into<String>) -> Self {
        self.msg.server_id = server_id.into();
        self
    }
    
    /// 设置消息类型
    pub fn msg_type(mut self, msg_type: MsgType) -> Self {
        self.msg.msg_type = msg_type as i32;
        self
    }
    
    /// 设置内容类型
    pub fn content_type(mut self, content_type: ContentType) -> Self {
        self.msg.content_type = content_type as i32;
        self
    }
    
    /// 设置消息内容
    pub fn content(mut self, content: impl Into<Vec<u8>>) -> Self {
        self.msg.content = content.into();
        self
    }
    
    /// 设置文本内容
    pub fn text_content(mut self, text: impl Into<String>) -> Self {
        self.msg.content = text.into().into_bytes();
        self.msg.content_type = ContentType::Text as i32;
        self
    }
    
    /// 设置创建时间
    pub fn create_time(mut self, create_time: i64) -> Self {
        self.msg.create_time = create_time;
        self
    }
    
    /// 设置发送时间
    pub fn send_time(mut self, send_time: i64) -> Self {
        self.msg.send_time = send_time;
        self
    }
    
    /// 设置接收序列号
    pub fn seq(mut self, seq: i64) -> Self {
        self.msg.seq = seq;
        self
    }
    
    /// 设置发送序列号
    pub fn send_seq(mut self, send_seq: i64) -> Self {
        self.msg.send_seq = send_seq;
        self
    }
    
    /// 设置平台类型
    pub fn platform(mut self, platform: crate::message::PlatformType) -> Self {
        self.msg.platform = platform as i32;
        self
    }
    
    /// 设置发送者头像
    pub fn avatar(mut self, avatar: impl Into<String>) -> Self {
        self.msg.avatar = avatar.into();
        self
    }
    
    /// 设置发送者昵称
    pub fn nickname(mut self, nickname: impl Into<String>) -> Self {
        self.msg.nickname = nickname.into();
        self
    }
    
    /// 设置关联消息ID
    pub fn related_msg_id(mut self, related_msg_id: impl Into<String>) -> Self {
        self.msg.related_msg_id = Some(related_msg_id.into());
        self
    }
    
    /// 设置已读状态
    pub fn is_read(mut self, is_read: bool) -> Self {
        self.msg.is_read = is_read;
        self
    }
    
    /// 构建单聊消息
    pub fn build_private_message(self) -> Result<Msg, String> {
        let mut msg = self.msg;
        msg.msg_type = MsgType::SingleMsg as i32;
        msg.group_id.clear(); // 确保单聊消息没有群组ID
        msg.validate()?;
        Ok(msg)
    }
    
    /// 构建群聊消息
    pub fn build_group_message(self) -> Result<Msg, String> {
        let mut msg = self.msg;
        msg.msg_type = MsgType::GroupMsg as i32;
        if msg.group_id.is_empty() {
            return Err("群聊消息必须设置群组ID".to_string());
        }
        msg.validate()?;
        Ok(msg)
    }
    
    /// 构建系统消息
    pub fn build_system_message(self, msg_type: MsgType) -> Result<Msg, String> {
        let mut msg = self.msg;
        msg.msg_type = msg_type as i32;
        msg.validate()?;
        Ok(msg)
    }
    
    /// 构建消息（不验证类型）
    pub fn build(self) -> Result<Msg, String> {
        self.msg.validate()?;
        Ok(self.msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MsgType, ContentType, PlatformType};
    
    #[test]
    fn test_msg_ext_private_message() {
        let msg = MsgBuilder::new()
            .send_id("user1")
            .receiver_id("user2")
            .text_content("Hello")
            .send_time(1234567890)
            .build_private_message()
            .unwrap();
        
        assert!(msg.is_private_message());
        assert!(!msg.is_group_message());
        assert_eq!(msg.get_conversation_id(), "user2");
        assert!(msg.is_text_message());
        assert!(!msg.is_system_message());
        assert!(msg.should_store_to_history());
    }
    
    #[test]
    fn test_msg_ext_group_message() {
        let msg = MsgBuilder::new()
            .send_id("user1")
            .receiver_id("user2")
            .group_id("group1")
            .text_content("Hello group")
            .send_time(1234567890)
            .build_group_message()
            .unwrap();
        
        assert!(msg.is_group_message());
        assert!(!msg.is_private_message());
        assert_eq!(msg.get_conversation_id(), "group1");
        assert!(msg.is_text_message());
        assert!(!msg.is_system_message());
    }
    
    #[test]
    fn test_msg_validation() {
        // 测试空发送者ID
        let result = MsgBuilder::new()
            .receiver_id("user2")
            .text_content("Hello")
            .send_time(1234567890)
            .build();
        assert!(result.is_err());
        
        // 测试群聊消息缺少群组ID
        let result = MsgBuilder::new()
            .send_id("user1")
            .receiver_id("user2")
            .text_content("Hello")
            .send_time(1234567890)
            .build_group_message();
        assert!(result.is_err());
    }
    
    #[test]
    fn test_message_summary() {
        let msg = MsgBuilder::new()
            .send_id("user1")
            .receiver_id("user2")
            .text_content("This is a very long message that should be truncated in the summary")
            .send_time(1234567890)
            .build_private_message()
            .unwrap();
        
        let summary = msg.get_summary();
        assert!(summary.len() <= 53); // 50 chars + "..."
        assert!(summary.contains("..."));
    }
} 