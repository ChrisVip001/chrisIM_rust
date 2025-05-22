
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

-- 双向好友关系表 (每个关系存储两条记录)
CREATE TABLE friend_relation
(
    id         VARCHAR(36) PRIMARY KEY,                       -- 关系记录ID (主键)
    user_id    VARCHAR(36) NOT NULL,                          -- 用户ID
    friend_id  VARCHAR(36) NOT NULL,                          -- 好友用户ID
    group_id   VARCHAR(64),                                   -- 所属分组ID (可为空)
    remark     VARCHAR(64)          DEFAULT '',               -- 好友备注名 (最多64字符)
    status     SMALLINT    NOT NULL DEFAULT 1,                -- 关系状态: 1-正常 2-拉黑
    created_at TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP -- 关系建立时间
    updated_at TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP -- 关系修改时间
);

-- 唯一约束: 防止重复添加同一好友
CREATE UNIQUE INDEX uk_user_friend ON friend_relation (user_id, friend_id);

-- 索引优化
CREATE INDEX idx_user_id ON friend_relation (user_id); -- 加速用户查询好友列表
CREATE INDEX idx_friend_id ON friend_relation (friend_id);
-- 加速反向关系查询

-- 表注释
COMMENT ON TABLE friend_relation IS '双向好友关系表';

-- 字段注释
COMMENT ON COLUMN friend_relation.user_id IS '关系所属的用户ID';
COMMENT ON COLUMN friend_relation.friend_id IS '被添加为好友的用户ID';
COMMENT ON COLUMN friend_relation.group_id IS '好友所在分组的ID (关联friend_group表)';
COMMENT ON COLUMN friend_relation.remark IS '用户为好友设置的备注名称';
COMMENT ON COLUMN friend_relation.status IS '关系状态: 1-正常好友 2-已拉黑';
COMMENT ON COLUMN friend_relation.created_at IS '好友关系的建立时间';
COMMENT ON COLUMN friend_relation.updated_at IS '好友关系的修改时间';


alter table friendships
    drop constraint fk_friend_id;
alter table friendships
    drop constraint fk_user_id;


-- 为friendships表添加拒绝理由字段
ALTER TABLE friendships ADD COLUMN reject_reason VARCHAR(255);

-- 为拒绝理由字段添加注释
COMMENT ON COLUMN friendships.reject_reason IS '好友请求拒绝理由';