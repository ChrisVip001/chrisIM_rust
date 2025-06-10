-- 修复分区表以继承新的列结构
-- 执行日期: 2025-01-05
-- 说明: 删除现有分区表并重新创建以继承主表的新列

-- 获取现有分区表并删除它们
DO $$
DECLARE
    partition_name TEXT;
    partition_range_start BIGINT;
    partition_range_end BIGINT;
    year_week_num TEXT;
    current_week_start TIMESTAMP;
BEGIN
    -- 删除所有现有的消息分区表
    FOR partition_name IN 
        SELECT tablename 
        FROM pg_tables 
        WHERE tablename LIKE 'messages_%' 
          AND schemaname = 'public'
    LOOP
        EXECUTE format('DROP TABLE IF EXISTS %s', partition_name);
        RAISE NOTICE '已删除分区表: %', partition_name;
    END LOOP;
    
    -- 重新创建分区表（从当前周开始，创建未来52周的分区）
    current_week_start := DATE_TRUNC('week', NOW());
    FOR i IN 0..51 LOOP
        -- 计算每个分区的开始和结束时间戳（毫秒）
        partition_range_start := (EXTRACT(EPOCH FROM current_week_start + (i * INTERVAL '1 week')) * 1000)::BIGINT;
        partition_range_end := (EXTRACT(EPOCH FROM current_week_start + ((i + 1) * INTERVAL '1 week')) * 1000)::BIGINT;
        -- 年份和周数标识
        year_week_num := TO_CHAR(current_week_start + (i * INTERVAL '1 week'), 'IYYY_IW');

        -- 创建分区表（会自动继承主表的所有列）
        EXECUTE format('CREATE TABLE messages_%s PARTITION OF messages FOR VALUES FROM (%s) TO (%s)', 
                      year_week_num, partition_range_start, partition_range_end);
        RAISE NOTICE '已创建分区表: messages_% (范围: % - %)', year_week_num, partition_range_start, partition_range_end;
    END LOOP;
END
$$;

-- 验证分区表结构
SELECT 
    schemaname,
    tablename,
    'messages' as parent_table
FROM pg_tables 
WHERE tablename LIKE 'messages_%' 
  AND schemaname = 'public'
ORDER BY tablename;

-- 验证分区表继承了新的列
SELECT 
    table_name,
    column_name,
    data_type
FROM information_schema.columns 
WHERE table_name LIKE 'messages_%'
  AND column_name IN ('is_revoked', 'revoke_time', 'revoked_by', 'related_msg_id', 
                     'forward_comment', 'is_forwarded', 'is_reply', 'updated_at',
                     'create_time', 'seq', 'send_seq', 'is_read', 'group_id', 'avatar', 'nickname')
ORDER BY table_name, column_name; 