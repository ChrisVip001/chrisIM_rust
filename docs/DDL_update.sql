----------------------------250516  Start----------------------------

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

alter table friendships
drop constraint fk_friend_id;
alter table friendships
drop constraint fk_user_id;


-- 为friendships表添加拒绝理由字段
ALTER TABLE friendships ADD COLUMN reject_reason VARCHAR(255);

-- 为拒绝理由字段添加注释
COMMENT ON COLUMN friendships.reject_reason IS '好友请求拒绝理由';

----------------------------250516  End----------------------------
----------------------------250521  Start----------------------------
alter table group_messages
drop constraint check_content_type;
----------------------------250521  End----------------------------
----------------------------250526  Start----------------------------

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

----------------------------250526  End----------------------------
----------------------------250530  Start----------------------------

-- 修改现有的群组表，添加额外字段
ALTER TABLE groups ADD COLUMN max_members INTEGER NOT NULL DEFAULT 500; -- 群组最大成员数量，默认500人
ALTER TABLE groups ADD COLUMN announcement_id VARCHAR(36); -- 当前群公告ID
ALTER TABLE groups ADD COLUMN join_mode VARCHAR(10) NOT NULL DEFAULT 'APPROVAL' CHECK (join_mode IN ('OPEN', 'APPROVAL', 'INVITE_ONLY')); -- 加入模式：开放、审批、仅邀请
ALTER TABLE groups ADD COLUMN qrcode_id VARCHAR(36); -- 当前群二维码ID
ALTER TABLE groups ADD COLUMN group_type VARCHAR(10) NOT NULL DEFAULT 'NORMAL' CHECK (group_type IN ('NORMAL', 'SUPER')); -- 群类型：普通群、超级群


-- 为现有表添加注释
COMMENT ON TABLE groups IS '群组表';
COMMENT ON COLUMN groups.id IS '群组ID';
COMMENT ON COLUMN groups.name IS '群组名称';
COMMENT ON COLUMN groups.description IS '群组描述';
COMMENT ON COLUMN groups.avatar_url IS '群组头像URL';
COMMENT ON COLUMN groups.owner_id IS '群主ID';
COMMENT ON COLUMN groups.created_at IS '创建时间';
COMMENT ON COLUMN groups.updated_at IS '更新时间';
COMMENT ON COLUMN groups.max_members IS '群组最大成员数量，默认500人';
COMMENT ON COLUMN groups.announcement_id IS '当前群公告ID';
COMMENT ON COLUMN groups.join_mode IS '加入模式：OPEN(开放)、APPROVAL(审批)、INVITE_ONLY(仅邀请)';
COMMENT ON COLUMN groups.qrcode_id IS '当前群二维码ID';
COMMENT ON COLUMN groups.group_type IS '群类型：NORMAL(普通群)、SUPER(超级群/大群)';

COMMENT ON TABLE group_members IS '群组成员表';
COMMENT ON COLUMN group_members.id IS '成员记录ID';
COMMENT ON COLUMN group_members.group_id IS '群组ID';
COMMENT ON COLUMN group_members.user_id IS '用户ID';
COMMENT ON COLUMN group_members.role IS '角色：MEMBER(普通成员)、ADMIN(管理员)、OWNER(群主)';
COMMENT ON COLUMN group_members.joined_at IS '加入时间';

----------------------------250530  End----------------------------
----------------------------250531  Start----------------------------

-- 在用户表中添加个性签名字段
ALTER TABLE users ADD COLUMN sign VARCHAR(100);

-- 添加字段注释
COMMENT ON COLUMN users.sign IS '用户个性签名';

-- 为user_config表添加是否展示手机号字段
ALTER TABLE user_config ADD COLUMN show_phone int4 DEFAULT 2;

-- 添加字段注释
COMMENT ON COLUMN user_config.show_phone IS '是否展示手机号(1-是，2-否)';

ALTER TABLE group_members
DROP CONSTRAINT check_role;
COMMENT ON COLUMN group_members.role IS '角色：0：MEMBER(普通成员)、1：ADMIN(管理员)、2：OWNER(群主)';

alter table group_members alter column role drop default;
----------------------------250531  End----------------------------

----------------------------250606  Start----------------------------

-- 修改群成员设置表，添加新字段
ALTER TABLE group_member_settings
ADD COLUMN remark VARCHAR(255), -- 群备注
ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0, -- 群置顶，1-是，0-否
ADD COLUMN recall_notification INTEGER NOT NULL DEFAULT 0; -- 群撤回通知，1-是，0-否

COMMENT ON COLUMN group_member_settings.remark IS '群备注';
COMMENT ON COLUMN group_member_settings.is_pinned IS '群置顶，1-是，0-否';
COMMENT ON COLUMN group_member_settings.recall_notification IS '群撤回通知，1-是，0-否';

----------------------------250606  End----------------------------