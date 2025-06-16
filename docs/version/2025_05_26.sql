ALTER TABLE users
    ADD COLUMN custom_id VARCHAR(20) NOT NULL
        CONSTRAINT unique_custom_id unique;

COMMENT
ON COLUMN users.custom_id
IS '用户自定义ID';


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