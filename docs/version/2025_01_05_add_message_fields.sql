-- 为messages表添加缺失的字段
-- 执行日期: 2025-06-10
-- 说明: 添加撤回、转发、回复等相关字段支持

-- 为主表添加字段
ALTER TABLE messages
    ADD COLUMN IF NOT EXISTS create_time     BIGINT    DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS seq             BIGINT    DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS send_seq        BIGINT    DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS is_read         BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS group_id        VARCHAR   DEFAULT '',
    ADD COLUMN IF NOT EXISTS avatar          VARCHAR   DEFAULT '',
    ADD COLUMN IF NOT EXISTS nickname        VARCHAR   DEFAULT '',
    ADD COLUMN IF NOT EXISTS is_revoked      BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS revoke_time     BIGINT    DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS revoked_by      VARCHAR   DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS related_msg_id  VARCHAR   DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS forward_comment TEXT      DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS is_forwarded    BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS is_reply        BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS updated_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP;

-- 为所有现有分区表添加字段
DO
$$
    DECLARE
        partition_name TEXT;
    BEGIN
        -- 获取所有messages分区表
        FOR partition_name IN
            SELECT schemaname || '.' || tablename
            FROM pg_tables
            WHERE tablename LIKE 'messages_%'
              AND schemaname = 'public'
            LOOP
                -- 为每个分区表添加字段
                EXECUTE format('
            ALTER TABLE %s 
            ADD COLUMN IF NOT EXISTS create_time BIGINT DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS seq BIGINT DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS send_seq BIGINT DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS is_read BOOLEAN NOT NULL DEFAULT FALSE,
            ADD COLUMN IF NOT EXISTS group_id VARCHAR DEFAULT '''',
            ADD COLUMN IF NOT EXISTS avatar VARCHAR DEFAULT '''',
            ADD COLUMN IF NOT EXISTS nickname VARCHAR DEFAULT '''',
            ADD COLUMN IF NOT EXISTS is_revoked BOOLEAN NOT NULL DEFAULT FALSE,
            ADD COLUMN IF NOT EXISTS revoke_time BIGINT DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS revoked_by VARCHAR DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS related_msg_id VARCHAR DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS forward_comment TEXT DEFAULT NULL,
            ADD COLUMN IF NOT EXISTS is_forwarded BOOLEAN NOT NULL DEFAULT FALSE,
            ADD COLUMN IF NOT EXISTS is_reply BOOLEAN NOT NULL DEFAULT FALSE,
            ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        ', partition_name);
            END LOOP;
    END
$$;

-- 添加字段注释
COMMENT ON COLUMN messages.create_time IS '消息创建时间（客户端时间戳）';
COMMENT ON COLUMN messages.seq IS '接收者序列号，用于消息排序和去重';
COMMENT ON COLUMN messages.send_seq IS '发送序列号，用于发送者的消息排序';
COMMENT ON COLUMN messages.is_read IS '是否已读';
COMMENT ON COLUMN messages.group_id IS '群组ID（群聊消息时使用）';
COMMENT ON COLUMN messages.avatar IS '发送者头像URL';
COMMENT ON COLUMN messages.nickname IS '发送者昵称';
COMMENT ON COLUMN messages.is_revoked IS '是否已撤回';
COMMENT ON COLUMN messages.revoke_time IS '撤回时间戳（毫秒）';
COMMENT ON COLUMN messages.revoked_by IS '撤回者用户ID';
COMMENT ON COLUMN messages.related_msg_id IS '关联消息ID（用于回复、引用等）';
COMMENT ON COLUMN messages.forward_comment IS '转发时的附加评论';
COMMENT ON COLUMN messages.is_forwarded IS '是否为转发消息';
COMMENT ON COLUMN messages.is_reply IS '是否为回复消息';
COMMENT ON COLUMN messages.updated_at IS '消息更新时间';

-- 创建索引以提高查询性能
CREATE INDEX IF NOT EXISTS idx_messages_seq ON messages (seq);
CREATE INDEX IF NOT EXISTS idx_messages_send_seq ON messages (send_seq);
CREATE INDEX IF NOT EXISTS idx_messages_is_read ON messages (is_read);
CREATE INDEX IF NOT EXISTS idx_messages_group_id ON messages (group_id);
CREATE INDEX IF NOT EXISTS idx_messages_is_revoked ON messages (is_revoked);
CREATE INDEX IF NOT EXISTS idx_messages_related_msg_id ON messages (related_msg_id);
CREATE INDEX IF NOT EXISTS idx_messages_revoked_by ON messages (revoked_by);

-- 为未来的分区表创建默认模板（需要手动执行以下模板创建新分区时）
/*
模板SQL（创建新分区时使用）:
CREATE TABLE messages_YYYY_WW PARTITION OF messages 
FOR VALUES FROM (start_timestamp) TO (end_timestamp);

-- 分区表会自动继承主表的列定义和约束
*/

-- 验证字段添加是否成功
SELECT column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE table_name = 'messages'
  AND column_name IN ('create_time', 'seq', 'send_seq', 'is_read', 'group_id', 'avatar', 'nickname',
                      'is_revoked', 'revoke_time', 'revoked_by', 'related_msg_id',
                      'forward_comment', 'is_forwarded', 'is_reply', 'updated_at')
ORDER BY column_name;


-- 添加sequence表记录用户发送和接收的序列号
CREATE TABLE sequence
(
    user_id      VARCHAR PRIMARY KEY,
    send_max_seq BIGINT NOT NULL DEFAULT 0,
    rec_max_seq  BIGINT NOT NULL DEFAULT 0
);