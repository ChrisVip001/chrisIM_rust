-- =============================================================================
-- IM 消息存储 DDL —— 统一读扩散 + 元数据/内容分离
-- =============================================================================
-- 设计前提：面向高并发，不对旧结构做任何兼容。
--
-- 四条硬规则：
--   1. 一条消息只写一次。写入成本与群规模无关：单人聊和十万人群都是 1 行元数据。
--   2. 拉取走 (conversation_id, seq) 主键扫描，恒定命中单个分区、单段 B-tree。
--   3. 已读是游标，标记已读的写入成本恒定 1 行，与群成员数无关。
--   4. 元数据与内容分离：列表只读窄行，正文按需批量取。
--
-- 与「写扩散收件箱」的根本差别：不再有 user_id 维度的消息副本，也就没有扇出。
-- 客户端按会话增量拉取（conversation_id + seq 游标），而不是按 user 全局序列号。
--
-- conversation_id 约定（应用层用同一函数生成，保证两端一致）：
--   单聊  d:{较小的 user_id}:{较大的 user_id}
--   群聊  g:{group_id}
-- 前缀避免单聊与群聊 ID 空间重叠。
-- =============================================================================

BEGIN;

-- -----------------------------------------------------------------------------
-- 1. 会话：序号水位 + 会话列表排序依据
-- -----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS conversation
(
    conversation_id VARCHAR     NOT NULL PRIMARY KEY,
    conv_type       SMALLINT    NOT NULL,
    -- 号段水位：Redis 号段预留到的上界，用于 Redis 丢失后重建。
    -- 会因为领号段而跳号，不能直接当「最后一条消息」使用。
    last_seq        BIGINT      NOT NULL DEFAULT 0,
    -- 实际最后一条消息的 seq，未读数以它为基准
    last_msg_seq    BIGINT      NOT NULL DEFAULT 0,
    last_msg_time   BIGINT      NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ck_conversation_type CHECK (conv_type IN (1, 2))
);

COMMENT ON TABLE  conversation IS '会话元信息：会话级序号水位与最后消息时间';
COMMENT ON COLUMN conversation.conv_type IS '1=单聊，2=群聊';
COMMENT ON COLUMN conversation.last_seq IS 'Redis 号段的持久化水位（已预留上界，会跳号）';
COMMENT ON COLUMN conversation.last_msg_seq IS '实际最后一条消息的 seq，未读数基准';
COMMENT ON COLUMN conversation.last_msg_time IS '最后一条消息时间，会话列表排序依据（热数据由 Redis ZSET 承担）';

-- 单聊会话 ID 生成：保证两侧计算结果一致
CREATE OR REPLACE FUNCTION single_conversation_id(a VARCHAR, b VARCHAR)
RETURNS VARCHAR AS $$
BEGIN
    IF a <= b THEN
        RETURN 'd:' || a || ':' || b;
    END IF;
    RETURN 'd:' || b || ':' || a;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

COMMENT ON FUNCTION single_conversation_id(VARCHAR, VARCHAR) IS '生成单聊会话ID，两侧调用结果必须一致';

-- -----------------------------------------------------------------------------
-- 2. 消息元数据：唯一的消息级索引，恒定一行
-- -----------------------------------------------------------------------------
-- 这是整个设计的关键表：窄行 + 主键即查询路径。
-- 刻意不存 avatar / nickname —— 从用户缓存取，既让行更窄，
-- 也避免用户改昵称后历史消息显示旧值。
CREATE TABLE IF NOT EXISTS msg_meta
(
    conversation_id VARCHAR     NOT NULL,
    seq             BIGINT      NOT NULL,
    server_id       VARCHAR     NOT NULL,
    send_id         VARCHAR     NOT NULL,
    msg_type        INT         NOT NULL DEFAULT 0,
    content_type    INT         NOT NULL DEFAULT 0,
    send_time       BIGINT      NOT NULL,
    content_size    INT         NOT NULL DEFAULT 0,
    -- 小于阈值的正文内联（走 TOAST，小值在行内、大值自动外移），
    -- 换来「拉列表即得正文」，省掉一次回表
    inline_content  BYTEA,
    -- 只放无法从其他服务取到的字段：related_msg_id / forward_comment /
    -- is_forwarded / is_reply / platform
    detail          JSONB       NOT NULL DEFAULT '{}'::jsonb,
    is_revoked      BOOLEAN     NOT NULL DEFAULT FALSE,
    revoke_time     BIGINT      NOT NULL DEFAULT 0,
    revoked_by      VARCHAR     NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 主键即「按会话增量拉取」的扫描路径
    PRIMARY KEY (conversation_id, seq),
    -- 幂等键：分区表唯一约束必须包含分区键，故带上 conversation_id。
    -- 重放时 seq 会重新分配（留空洞，属正常），但不会写入重复消息。
    CONSTRAINT uk_msg_meta_server UNIQUE (conversation_id, server_id)
) PARTITION BY HASH (conversation_id);

COMMENT ON TABLE  msg_meta IS '消息元数据（读扩散，一条消息一行，与群规模无关）';
COMMENT ON COLUMN msg_meta.seq IS '会话内单调序号，由 Redis 号段分配';
COMMENT ON COLUMN msg_meta.inline_content IS '小于阈值的正文，避免额外回表；大内容存 msg_content';
COMMENT ON COLUMN msg_meta.detail IS '低频字段：related_msg_id/forward_comment/is_forwarded/is_reply/platform';

-- -----------------------------------------------------------------------------
-- 3. 消息正文：大内容单独存放，按需批量读取
-- -----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS msg_content
(
    server_id  VARCHAR     NOT NULL,
    content    BYTEA       NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (server_id)
) PARTITION BY HASH (server_id);

COMMENT ON TABLE msg_content IS '大消息正文（图片/文件/长文本），与元数据分离';

-- -----------------------------------------------------------------------------
-- 4. 用户会话状态：已读游标
-- -----------------------------------------------------------------------------
-- 未读数不落库：unread = conversation.last_seq - user_conversation.last_read_seq。
-- 这样「标记已读」只写 1 行，与群成员数无关；成员众多的群也不会产生 N 倍更新。
CREATE TABLE IF NOT EXISTS user_conversation
(
    user_id         VARCHAR     NOT NULL,
    conversation_id VARCHAR     NOT NULL,
    last_read_seq   BIGINT      NOT NULL DEFAULT 0,
    is_pinned       BOOLEAN     NOT NULL DEFAULT FALSE,
    is_muted        BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, conversation_id)
) PARTITION BY HASH (user_id);

COMMENT ON TABLE  user_conversation IS '用户会话状态：已读游标；（未读数由 conversation.last_seq 相减得出）';
COMMENT ON COLUMN user_conversation.last_read_seq IS '该用户在该会话已读到的最大 seq';

-- -----------------------------------------------------------------------------
-- 5. 分区
-- -----------------------------------------------------------------------------
-- 刻意不做时间维度的二级分区：
--   拉取路径是 (conversation_id, seq) 主键扫描，加时间分区会让一次
--   「拉最近 50 条」退化成跨 12 个月分区的 MergeAppend，得不偿失。
-- 历史数据通过归档任务按 seq 段迁移（见文末说明），而非 DROP PARTITION。
DO $$
DECLARE
    v INT;
BEGIN
    FOR v IN 0..31 LOOP
        EXECUTE format(
            'CREATE TABLE IF NOT EXISTS %I PARTITION OF msg_meta FOR VALUES WITH (MODULUS 32, REMAINDER %s)',
            format('msg_meta_p%s', lpad(v::text, 2, '0')), v
        );
    END LOOP;

    FOR v IN 0..15 LOOP
        EXECUTE format(
            'CREATE TABLE IF NOT EXISTS %I PARTITION OF msg_content FOR VALUES WITH (MODULUS 16, REMAINDER %s)',
            format('msg_content_p%s', lpad(v::text, 2, '0')), v
        );
    END LOOP;

    FOR v IN 0..31 LOOP
        EXECUTE format(
            'CREATE TABLE IF NOT EXISTS %I PARTITION OF user_conversation FOR VALUES WITH (MODULUS 32, REMAINDER %s)',
            format('user_conversation_p%s', lpad(v::text, 2, '0')), v
        );
    END LOOP;
END
$$;

-- -----------------------------------------------------------------------------
-- 6. 索引
-- -----------------------------------------------------------------------------
-- 会话列表按最后消息时间排序；主键已覆盖按 seq 翻页，这里只补时间维度
CREATE INDEX IF NOT EXISTS idx_msg_meta_conv_time
    ON msg_meta (conversation_id, send_time DESC);

-- 服务端会话列表兜底查询（热路径走 Redis ZSET，冷路径走这里）
CREATE INDEX IF NOT EXISTS idx_conversation_last_msg_time
    ON conversation (last_msg_time DESC);

-- 用户会话列表：按用户取全部会话，再 join conversation 排序
CREATE INDEX IF NOT EXISTS idx_user_conversation_user
    ON user_conversation (user_id);

COMMIT;

-- =============================================================================
-- 写入路径（恒定 3 次写，与群规模无关）
-- =============================================================================
-- 1) Redis 号段领 seq（号段用尽时异步回写 conversation.last_seq）
-- 2) INSERT INTO msg_meta (...) VALUES (...) ON CONFLICT DO NOTHING
--       —— 小正文内联写入；大正文走下一步
-- 3) INSERT INTO msg_content (server_id, content) VALUES (...) ON CONFLICT DO NOTHING
-- 4) UPDATE conversation SET last_seq = ..., last_msg_time = ... （号段批量回写，非每消息）
-- 5) 发送者自身 user_conversation 的 last_read_seq 可顺带推进（可选，1 行）
--
-- 万人群发一条消息：2 行写入。写扩散方案是 10001 行。
--
-- =============================================================================
-- 读取路径（恒定 1 次主键扫描 + 按需批量取正文）
-- =============================================================================
-- 增量拉取：
--   SELECT ... FROM msg_meta
--   WHERE conversation_id = $1 AND seq > $2
--   ORDER BY seq LIMIT $3;
-- 正文补取（仅当 inline_content IS NULL 且 content_size > 0）：
--   SELECT server_id, content FROM msg_content WHERE server_id = ANY($1);
--
-- 标记已读：UPDATE user_conversation SET last_read_seq = $3
--           WHERE user_id = $1 AND conversation_id = $2;
-- 未读数：  SELECT c.last_seq - uc.last_read_seq
--           FROM conversation c JOIN user_conversation uc USING (conversation_id)
--           WHERE uc.user_id = $1 AND uc.conversation_id = $2;
--
-- =============================================================================
-- 归档策略（替代按时间 DROP PARTITION）
-- =============================================================================
-- 后台任务按 conversation 逐个迁移：把 seq <= archive_before_seq 的元数据搬到
-- 归档表（同结构、不同表空间/存储），保留 conversation 中的水位。
-- 不做定期 DELETE：读扩散主键是 (conversation_id, seq)，时间分区会破坏扫描路径，
-- 而大表随机 DELETE 产生的死元组会拖垮 vacuum。
--
-- =============================================================================
-- 高可用要点（部署层，不在本脚本内）
-- =============================================================================
-- 1) PostgreSQL 一主多从 + Patroni/etcd 自动故障转移，同步复制至少 1 副本。
-- 2) 应用侧 PgBouncer（transaction pooling），每服务独立 database + user，
--    序号分配与消息读写使用独立连接池，避免长查询饿死热点写入。
-- 3) 历史拉取走只读副本；刚发出的消息由 Redis 最近消息缓存兜底，
--    规避复制延迟导致的「刚发就拉不到」。
-- 4) Redis 承担：号段分配、最近 N 条消息缓存、用户会话列表 ZSET。
-- 5) Kafka 按 conversation_id 分区，消费端并行（每分区一个 worker，批量提交位点）。
