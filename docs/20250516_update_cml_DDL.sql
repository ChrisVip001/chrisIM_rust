
-- 删除原有字符串约束
ALTER TABLE friendships
DROP CONSTRAINT check_status;

-- 更新默认值为 0 (对应原来的 'PENDING')
ALTER TABLE friendships
    ALTER COLUMN status SET DEFAULT 0;

ALTER TABLE friendships
ALTER COLUMN status TYPE VARCHAR(10)
        USING CASE status
                  WHEN 'PENDING' THEN '0'
                  WHEN 'ACCEPTED' THEN '1'
                  WHEN 'REJECTED' THEN '2'
                  WHEN 'BLOCKED' THEN '3'
END;



-- 添加新的整型约束
ALTER TABLE friendships
    ADD CONSTRAINT check_status CHECK (status IN ('0', '1', '2','3'));

-- 增加验证信息字段
ALTER TABLE friendships
    ADD COLUMN message varchar(255) DEFAULT '';