-- 完整的messages表DDL语句
-- 合并自DDL_init.sql、2025_01_05_fix_partitions.sql和2025_01_05_add_message_fields.sql
-- 执行日期: 2025-01-XX

-- 删除现有的messages表及其所有分区表
DO $$
DECLARE
    partition_name TEXT;
BEGIN
    -- 删除所有现有的消息分区表
    FOR partition_name IN 
        SELECT tablename 
        FROM pg_tables 
        WHERE tablename LIKE 'messages_%' 
          AND schemaname = 'public'
    LOOP
        EXECUTE format('DROP TABLE IF EXISTS %s CASCADE', partition_name);
        RAISE NOTICE '已删除分区表: %', partition_name;
    END LOOP;
    
    -- 删除主表
    DROP TABLE IF EXISTS messages CASCADE;
    RAISE NOTICE '已删除主表: messages';
END
$$;

-- 删除sequence表（如果存在）
DROP TABLE IF EXISTS sequence CASCADE;

-- 创建完整的messages表（包含所有字段）
CREATE TABLE messages
(
    send_id          VARCHAR   NOT NULL,
    receiver_id      VARCHAR   NOT NULL,
    local_id         VARCHAR   NOT NULL,
    server_id        VARCHAR   NOT NULL,
    send_time        BIGINT    NOT NULL,
    msg_type         INT,
    content_type     INT,
    content          BYTEA,
    platform         INT,
    create_time      BIGINT    DEFAULT NULL,
    seq              BIGINT    DEFAULT NULL,
    send_seq         BIGINT    DEFAULT NULL,
    is_read          BOOLEAN   NOT NULL DEFAULT FALSE,
    group_id         VARCHAR   DEFAULT '',
    avatar           VARCHAR   DEFAULT '',
    nickname         VARCHAR   DEFAULT '',
    is_revoked       BOOLEAN   NOT NULL DEFAULT FALSE,
    revoke_time      BIGINT    DEFAULT NULL,
    revoked_by       VARCHAR   DEFAULT NULL,
    related_msg_id   VARCHAR   DEFAULT NULL,
    forward_comment  TEXT      DEFAULT NULL,
    is_forwarded     BOOLEAN   NOT NULL DEFAULT FALSE,
    is_reply         BOOLEAN   NOT NULL DEFAULT FALSE,
    updated_at       TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (send_id, server_id, send_time)
) PARTITION BY RANGE (send_time);

-- 添加字段注释
COMMENT ON TABLE messages IS '消息表（分区表）';
COMMENT ON COLUMN messages.send_id IS '发送者ID';
COMMENT ON COLUMN messages.receiver_id IS '接收者ID';
COMMENT ON COLUMN messages.local_id IS '本地消息ID';
COMMENT ON COLUMN messages.server_id IS '服务器消息ID';
COMMENT ON COLUMN messages.send_time IS '发送时间戳（毫秒）';
COMMENT ON COLUMN messages.msg_type IS '消息类型';
COMMENT ON COLUMN messages.content_type IS '内容类型';
COMMENT ON COLUMN messages.content IS '消息内容（二进制）';
COMMENT ON COLUMN messages.platform IS '平台标识';
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

-- 创建分区表（从当前周开始，创建未来52周的分区）
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

        -- 创建分区表（会自动继承主表的所有列）
        EXECUTE format('CREATE TABLE messages_%s PARTITION OF messages FOR VALUES FROM (%s) TO (%s)', 
                      year_week_num, week_start_ms, week_end_ms);
        RAISE NOTICE '已创建分区表: messages_% (范围: % - %)', year_week_num, week_start_ms, week_end_ms;
    END LOOP;
END
$$;

-- 创建索引以提高查询性能
CREATE INDEX IF NOT EXISTS idx_messages_seq ON messages (seq);
CREATE INDEX IF NOT EXISTS idx_messages_send_seq ON messages (send_seq);
CREATE INDEX IF NOT EXISTS idx_messages_is_read ON messages (is_read);
CREATE INDEX IF NOT EXISTS idx_messages_group_id ON messages (group_id);
CREATE INDEX IF NOT EXISTS idx_messages_is_revoked ON messages (is_revoked);
CREATE INDEX IF NOT EXISTS idx_messages_related_msg_id ON messages (related_msg_id);
CREATE INDEX IF NOT EXISTS idx_messages_revoked_by ON messages (revoked_by);

-- 创建sequence表记录用户发送和接收的序列号
CREATE TABLE sequence
(
    user_id      VARCHAR PRIMARY KEY,
    send_max_seq BIGINT NOT NULL DEFAULT 0,
    rec_max_seq  BIGINT NOT NULL DEFAULT 0
);

COMMENT ON TABLE sequence IS '用户消息序列号表';
COMMENT ON COLUMN sequence.user_id IS '用户ID';
COMMENT ON COLUMN sequence.send_max_seq IS '用户最大发送序列号';
COMMENT ON COLUMN sequence.rec_max_seq IS '用户最大接收序列号';

-- 验证表结构
SELECT 
    schemaname,
    tablename,
    'messages' as parent_table
FROM pg_tables 
WHERE tablename LIKE 'messages_%' 
  AND schemaname = 'public'
ORDER BY tablename;

-- 验证字段是否创建成功
SELECT column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE table_name = 'messages'
ORDER BY ordinal_position;

RAISE NOTICE 'messages表及其分区表创建完成！'; 