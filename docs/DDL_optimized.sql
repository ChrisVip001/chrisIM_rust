-- 优化后的消息表结构
-- 保持单表设计，但增强语义清晰度和查询性能

-- 删除旧的消息表（如果存在）
DROP TABLE IF EXISTS messages CASCADE;

-- 创建优化后的消息表
CREATE TABLE messages (
    -- 基础字段
    send_id      VARCHAR(36) NOT NULL,   -- 发送者用户ID
    receiver_id  VARCHAR(36) NOT NULL,   -- 接收者ID（单聊时是用户ID，群聊时也是用户ID）
    group_id     VARCHAR(36),            -- 群组ID（群聊时使用，单聊时为NULL）
    local_id     VARCHAR(36) NOT NULL,   -- 客户端本地消息ID
    server_id    VARCHAR(36) NOT NULL,   -- 服务器生成的全局唯一消息ID
    
    -- 时间戳字段
    create_time  BIGINT NOT NULL,        -- 消息创建时间（客户端时间戳）
    send_time    BIGINT NOT NULL,        -- 消息发送时间（服务器时间戳）
    
    -- 序列号字段
    seq          BIGINT NOT NULL,        -- 接收者序列号，用于消息排序和去重
    send_seq     BIGINT NOT NULL DEFAULT 0, -- 发送者序列号，用于发送者的消息排序
    
    -- 消息类型和内容
    msg_type     INT NOT NULL,           -- 消息类型（参考MsgType枚举）
    content_type INT NOT NULL,           -- 内容类型（参考ContentType枚举）
    content      BYTEA,                  -- 消息内容
    
    -- 状态字段
    is_read      BOOLEAN NOT NULL DEFAULT FALSE, -- 是否已读
    platform     INT NOT NULL,           -- 发送者平台类型
    
    -- 发送者信息（冗余存储，提高查询性能）
    avatar       VARCHAR(255),           -- 发送者头像URL
    nickname     VARCHAR(100),           -- 发送者昵称
    
    -- 关联消息ID（用于回复、引用等功能）
    related_msg_id VARCHAR(36),          -- 关联的消息ID
    
    -- 计算字段：是否为群聊消息
    is_group_msg BOOLEAN GENERATED ALWAYS AS (group_id IS NOT NULL) STORED,
    
    -- 主键：发送者ID + 服务器消息ID + 发送时间
    PRIMARY KEY (send_id, server_id, send_time),
    
    -- 唯一约束：确保服务器消息ID全局唯一
    CONSTRAINT uk_server_id UNIQUE (server_id)
) PARTITION BY RANGE (send_time);

-- 创建分区表（按周分区，覆盖一年）
DO $$
DECLARE
    week_start_ms BIGINT;
    week_end_ms BIGINT;
    current_week_start TIMESTAMP;
    year_week_num VARCHAR(10);
BEGIN
    -- 获取当前时间戳所在周的开始时间（使用毫秒）
    current_week_start := DATE_TRUNC('week', NOW());
    FOR i IN 0..51 LOOP
        -- 计算每个分区的开始和结束时间戳（毫秒）
        week_start_ms := (EXTRACT(EPOCH FROM current_week_start + (i * INTERVAL '1 week')) * 1000)::BIGINT;
        week_end_ms := (EXTRACT(EPOCH FROM current_week_start + ((i + 1) * INTERVAL '1 week')) * 1000)::BIGINT;
        -- 年份和周数标识
        year_week_num := TO_CHAR(current_week_start + (i * INTERVAL '1 week'), 'IYYY_IW');

        -- 创建分区表
        EXECUTE 'CREATE TABLE IF NOT EXISTS messages_' || year_week_num || ' PARTITION OF messages FOR VALUES FROM (' || week_start_ms || ') TO (' || week_end_ms || ')';
    END LOOP;
END
$$;

-- ================================
-- 索引优化
-- ================================

-- 1. 单聊消息专用索引
-- 用于查询用户的单聊消息
CREATE INDEX idx_private_messages_receiver ON messages (receiver_id, send_time DESC) 
WHERE group_id IS NULL;

-- 用于查询用户发送的单聊消息
CREATE INDEX idx_private_messages_sender ON messages (send_id, send_time DESC) 
WHERE group_id IS NULL;

-- 2. 群聊消息专用索引
-- 用于查询群组消息
CREATE INDEX idx_group_messages_group ON messages (group_id, send_time DESC) 
WHERE group_id IS NOT NULL;

-- 用于查询用户在群组中的消息
CREATE INDEX idx_group_messages_user ON messages (receiver_id, group_id, send_time DESC) 
WHERE group_id IS NOT NULL;

-- 3. 序列号相关索引
-- 用于序列号范围查询
CREATE INDEX idx_messages_seq ON messages (receiver_id, seq);
CREATE INDEX idx_messages_send_seq ON messages (send_id, send_seq);

-- 4. 消息类型索引
-- 用于按消息类型查询
CREATE INDEX idx_messages_type ON messages (msg_type, send_time DESC);

-- 5. 已读状态索引
-- 用于查询未读消息
CREATE INDEX idx_messages_unread ON messages (receiver_id, is_read, send_time DESC) 
WHERE is_read = FALSE;

-- 6. 服务器消息ID索引（已通过唯一约束创建）
-- 用于快速查找特定消息

-- 7. 关联消息索引
-- 用于查询回复链
CREATE INDEX idx_messages_related ON messages (related_msg_id) 
WHERE related_msg_id IS NOT NULL;

-- ================================
-- 表注释
-- ================================

COMMENT ON TABLE messages IS '消息表 - 统一存储单聊和群聊消息';
COMMENT ON COLUMN messages.send_id IS '发送者用户ID';
COMMENT ON COLUMN messages.receiver_id IS '接收者ID（单聊时是用户ID，群聊时是群成员ID）';
COMMENT ON COLUMN messages.group_id IS '群组ID（群聊消息时使用，单聊时为NULL）';
COMMENT ON COLUMN messages.local_id IS '客户端生成的本地消息ID';
COMMENT ON COLUMN messages.server_id IS '服务器生成的全局唯一消息ID';
COMMENT ON COLUMN messages.create_time IS '消息创建时间（客户端时间戳，毫秒）';
COMMENT ON COLUMN messages.send_time IS '消息发送时间（服务器时间戳，毫秒）';
COMMENT ON COLUMN messages.seq IS '接收者序列号，用于消息排序和去重';
COMMENT ON COLUMN messages.send_seq IS '发送者序列号，用于发送者的消息排序';
COMMENT ON COLUMN messages.msg_type IS '消息类型（参考MsgType枚举）';
COMMENT ON COLUMN messages.content_type IS '内容类型（参考ContentType枚举）';
COMMENT ON COLUMN messages.content IS '消息内容（二进制格式）';
COMMENT ON COLUMN messages.is_read IS '是否已读';
COMMENT ON COLUMN messages.platform IS '发送者平台类型';
COMMENT ON COLUMN messages.avatar IS '发送者头像URL（冗余存储）';
COMMENT ON COLUMN messages.nickname IS '发送者昵称（冗余存储）';
COMMENT ON COLUMN messages.related_msg_id IS '关联的消息ID（用于回复、引用等）';
COMMENT ON COLUMN messages.is_group_msg IS '是否为群聊消息（计算字段）';

-- ================================
-- 创建视图简化查询
-- ================================

-- 单聊消息视图
CREATE VIEW private_messages AS
SELECT 
    send_id,
    receiver_id,
    local_id,
    server_id,
    create_time,
    send_time,
    seq,
    send_seq,
    msg_type,
    content_type,
    content,
    is_read,
    platform,
    avatar,
    nickname,
    related_msg_id
FROM messages 
WHERE group_id IS NULL;

-- 群聊消息视图
CREATE VIEW group_messages AS
SELECT 
    send_id,
    receiver_id,
    group_id,
    local_id,
    server_id,
    create_time,
    send_time,
    seq,
    send_seq,
    msg_type,
    content_type,
    content,
    is_read,
    platform,
    avatar,
    nickname,
    related_msg_id
FROM messages 
WHERE group_id IS NOT NULL;

-- 用户消息统计视图
CREATE VIEW user_message_stats AS
SELECT 
    receiver_id as user_id,
    COUNT(*) as total_messages,
    COUNT(*) FILTER (WHERE is_read = FALSE) as unread_count,
    COUNT(*) FILTER (WHERE group_id IS NULL) as private_count,
    COUNT(*) FILTER (WHERE group_id IS NOT NULL) as group_count,
    MAX(send_time) as last_message_time
FROM messages 
GROUP BY receiver_id;

COMMENT ON VIEW private_messages IS '单聊消息视图';
COMMENT ON VIEW group_messages IS '群聊消息视图';
COMMENT ON VIEW user_message_stats IS '用户消息统计视图'; 