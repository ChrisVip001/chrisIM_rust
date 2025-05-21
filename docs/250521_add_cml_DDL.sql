-- 好友分组表
CREATE TABLE IF NOT EXISTS friend_group
(
    id         varchar(36) PRIMARY KEY,
    user_id    varchar(36) NOT NULL,
    group_name VARCHAR(50) NOT NULL,
    sort_order INT       DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
    );

COMMENT ON TABLE friend_group IS '好友分组表';
COMMENT ON COLUMN friend_group.id IS '分组ID';
COMMENT ON COLUMN friend_group.user_id IS '用户ID';
COMMENT ON COLUMN friend_group.group_name IS '分组名称';
COMMENT ON COLUMN friend_group.sort_order IS '排序顺序';
COMMENT ON COLUMN friend_group.created_at IS '创建时间';
COMMENT ON COLUMN friend_group.updated_at IS '更新时间';

CREATE INDEX idx_friend_group_user_id ON friend_group (user_id);

-- 好友分组关系表
CREATE TABLE IF NOT EXISTS friend_group_relation
(
    id         varchar(36) PRIMARY KEY,
    user_id    varchar(36) NOT NULL,
    friend_id  varchar(36) NOT NULL,
    group_id   varchar(36) NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uk_user_friend_group UNIQUE (user_id, friend_id, group_id)
    );

COMMENT ON TABLE friend_group_relation IS '好友分组关系表';
COMMENT ON COLUMN friend_group_relation.id IS '关系ID';
COMMENT ON COLUMN friend_group_relation.user_id IS '用户ID';
COMMENT ON COLUMN friend_group_relation.friend_id IS '好友ID';
COMMENT ON COLUMN friend_group_relation.group_id IS '分组ID';
COMMENT ON COLUMN friend_group_relation.created_at IS '创建时间';
COMMENT ON COLUMN friend_group_relation.updated_at IS '更新时间';