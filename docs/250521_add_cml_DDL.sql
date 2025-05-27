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

alter table group_messages
drop constraint check_content_type;

-- 添加好友类型字段到friend_relation表
ALTER TABLE friend_relation
    ADD COLUMN friend_type SMALLINT NOT NULL DEFAULT 0; -- 好友类型: 0-普通好友 1-官方账号 2-系统账号

-- 添加字段注释
COMMENT ON COLUMN friend_relation.friend_type IS '好友类型: 0-普通好友 1-官方账号 2-系统账号';

-- 添加星标和置顶字段到friend_relation表（使用整数类型）
ALTER TABLE friend_relation
    ADD COLUMN is_starred SMALLINT NOT NULL DEFAULT 0,
    ADD COLUMN is_top SMALLINT NOT NULL DEFAULT 0;

-- 添加字段注释
COMMENT ON COLUMN friend_relation.is_starred IS '是否星标好友: 0-不是星标 1-是星标';
COMMENT ON COLUMN friend_relation.is_top IS '是否置顶好友: 0-不是置顶 1-是置顶';

-- 创建组合索引，提高查询效率
CREATE INDEX idx_friend_relation_star_top ON friend_relation (is_starred, is_top);
