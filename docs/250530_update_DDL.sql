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


-- 群组设置表 (支持禁止群成员相互加好友等设置)
CREATE TABLE group_settings
(
    group_id                VARCHAR(36) PRIMARY KEY,                     -- 群组ID
    allow_member_friendship INTEGER     NOT NULL DEFAULT 1,           -- 是否允许群成员相互加好友，1-允许，0-不允许
    join_approval_required  INTEGER     NOT NULL DEFAULT 0,          -- 是否需要审批才能加入群组，1-需要，0-不需要
    only_admin_can_invite   INTEGER     NOT NULL DEFAULT 0,          -- 是否仅管理员可以邀请新成员，1-是，0-否
    only_admin_can_modify   INTEGER     NOT NULL DEFAULT 0,          -- 是否仅管理员可以修改群信息，1-是，0-否
    updated_at              TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP -- 最后更新时间
);
COMMENT ON TABLE group_settings IS '群组设置表';
COMMENT ON COLUMN group_settings.group_id IS '群组ID';
COMMENT ON COLUMN group_settings.allow_member_friendship IS '是否允许群成员相互加好友，1-允许，0-不允许';
COMMENT ON COLUMN group_settings.join_approval_required IS '是否需要审批才能加入群组，1-需要，0-不需要';
COMMENT ON COLUMN group_settings.only_admin_can_invite IS '是否仅管理员可以邀请新成员，1-是，0-否';
COMMENT ON COLUMN group_settings.only_admin_can_modify IS '是否仅管理员可以修改群信息，1-是，0-否';
COMMENT ON COLUMN group_settings.updated_at IS '最后更新时间';

CREATE TRIGGER update_group_settings_modtime
    BEFORE UPDATE
    ON group_settings
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();

-- 群公告表
CREATE TABLE group_announcements
(
    id         VARCHAR(36) PRIMARY KEY,                              -- 公告ID
    group_id   VARCHAR(36)  NOT NULL,                                -- 群组ID
    creator_id VARCHAR(36)  NOT NULL,                                -- 创建者ID
    content    TEXT         NOT NULL,                                -- 公告内容
    title      VARCHAR(255),                                         -- 公告标题
    is_pinned  INTEGER      NOT NULL DEFAULT 0,                  -- 是否置顶，1-置顶，0-不置顶
    created_at TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,      -- 创建时间
    updated_at TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP       -- 更新时间
);
COMMENT ON TABLE group_announcements IS '群公告表';
COMMENT ON COLUMN group_announcements.id IS '公告ID';
COMMENT ON COLUMN group_announcements.group_id IS '群组ID';
COMMENT ON COLUMN group_announcements.creator_id IS '创建者ID';
COMMENT ON COLUMN group_announcements.content IS '公告内容';
COMMENT ON COLUMN group_announcements.title IS '公告标题';
COMMENT ON COLUMN group_announcements.is_pinned IS '是否置顶，1-置顶，0-不置顶';
COMMENT ON COLUMN group_announcements.created_at IS '创建时间';
COMMENT ON COLUMN group_announcements.updated_at IS '更新时间';

CREATE INDEX idx_group_announcements_group_id ON group_announcements (group_id);
CREATE INDEX idx_group_announcements_is_pinned ON group_announcements (is_pinned);

CREATE TRIGGER update_group_announcements_modtime
    BEFORE UPDATE
    ON group_announcements
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();

-- 群置顶聊天表
CREATE TABLE group_pinned_messages
(
    id           VARCHAR(36) PRIMARY KEY,                            -- 置顶消息ID
    group_id     VARCHAR(36) NOT NULL,                               -- 群组ID
    message_id   VARCHAR(36) NOT NULL,                               -- 消息ID
    creator_id   VARCHAR(36) NOT NULL,                               -- 创建者ID
    created_at   TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP      -- 创建时间
);
COMMENT ON TABLE group_pinned_messages IS '群置顶聊天表';
COMMENT ON COLUMN group_pinned_messages.id IS '置顶消息ID';
COMMENT ON COLUMN group_pinned_messages.group_id IS '群组ID';
COMMENT ON COLUMN group_pinned_messages.message_id IS '消息ID';
COMMENT ON COLUMN group_pinned_messages.creator_id IS '创建者ID';
COMMENT ON COLUMN group_pinned_messages.created_at IS '创建时间';

CREATE INDEX idx_group_pinned_messages_group_id ON group_pinned_messages (group_id);

-- 群黑名单表
CREATE TABLE group_blacklist
(
    id           VARCHAR(36) PRIMARY KEY,                            -- 黑名单记录ID
    group_id     VARCHAR(36) NOT NULL,                               -- 群组ID
    user_id      VARCHAR(36) NOT NULL,                               -- 被拉黑用户ID
    creator_id   VARCHAR(36) NOT NULL,                               -- 操作者ID
    reason       TEXT,                                               -- 拉黑理由
    created_at   TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,     -- 创建时间
    CONSTRAINT unique_group_blacklist UNIQUE (group_id, user_id)
);
COMMENT ON TABLE group_blacklist IS '群黑名单表';
COMMENT ON COLUMN group_blacklist.id IS '黑名单记录ID';
COMMENT ON COLUMN group_blacklist.group_id IS '群组ID';
COMMENT ON COLUMN group_blacklist.user_id IS '被拉黑用户ID';
COMMENT ON COLUMN group_blacklist.creator_id IS '操作者ID';
COMMENT ON COLUMN group_blacklist.reason IS '拉黑理由';
COMMENT ON COLUMN group_blacklist.created_at IS '创建时间';

CREATE INDEX idx_group_blacklist_group_id ON group_blacklist (group_id);
CREATE INDEX idx_group_blacklist_user_id ON group_blacklist (user_id);

-- 群禁言表
CREATE TABLE group_mutes
(
    id           VARCHAR(36) PRIMARY KEY,                            -- 禁言记录ID
    group_id     VARCHAR(36) NOT NULL,                               -- 群组ID
    user_id      VARCHAR(36) NOT NULL,                               -- 被禁言用户ID，NULL表示全员禁言
    creator_id   VARCHAR(36) NOT NULL,                               -- 操作者ID
    reason       TEXT,                                               -- 禁言理由
    mute_until   TIMESTAMP,                                          -- 禁言结束时间
    is_permanent INTEGER     NOT NULL DEFAULT 0,                 -- 是否永久禁言，1-是，0-否
    created_at   TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,     -- 创建时间
    updated_at   TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,     -- 更新时间
    CONSTRAINT unique_group_mute UNIQUE (group_id, user_id)
);
COMMENT ON TABLE group_mutes IS '群禁言表';
COMMENT ON COLUMN group_mutes.id IS '禁言记录ID';
COMMENT ON COLUMN group_mutes.group_id IS '群组ID';
COMMENT ON COLUMN group_mutes.user_id IS '被禁言用户ID，NULL表示全员禁言';
COMMENT ON COLUMN group_mutes.creator_id IS '操作者ID';
COMMENT ON COLUMN group_mutes.reason IS '禁言理由';
COMMENT ON COLUMN group_mutes.mute_until IS '禁言结束时间';
COMMENT ON COLUMN group_mutes.is_permanent IS '是否永久禁言，1-是，0-否';
COMMENT ON COLUMN group_mutes.created_at IS '创建时间';
COMMENT ON COLUMN group_mutes.updated_at IS '更新时间';

CREATE INDEX idx_group_mutes_group_id ON group_mutes (group_id);
CREATE INDEX idx_group_mutes_user_id ON group_mutes (user_id);
CREATE INDEX idx_group_mutes_mute_until ON group_mutes (mute_until);

CREATE TRIGGER update_group_mutes_modtime
    BEFORE UPDATE
    ON group_mutes
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();

-- 群邀请表
CREATE TABLE group_invitations
(
    id           VARCHAR(36) PRIMARY KEY,                            -- 邀请记录ID
    group_id     VARCHAR(36) NOT NULL,                               -- 群组ID
    inviter_id   VARCHAR(36) NOT NULL,                               -- 邀请人ID
    invitee_id   VARCHAR(36) NOT NULL,                               -- 被邀请人ID
    status       VARCHAR(10) NOT NULL DEFAULT 'PENDING',             -- 邀请状态：待处理、已接受、已拒绝
    created_at   TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,     -- 创建时间
    updated_at   TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,     -- 更新时间
    CONSTRAINT check_invitation_status CHECK (status IN ('PENDING', 'ACCEPTED', 'REJECTED')),
    CONSTRAINT unique_group_invitation UNIQUE (group_id, invitee_id)
);
COMMENT ON TABLE group_invitations IS '群邀请表';
COMMENT ON COLUMN group_invitations.id IS '邀请记录ID';
COMMENT ON COLUMN group_invitations.group_id IS '群组ID';
COMMENT ON COLUMN group_invitations.inviter_id IS '邀请人ID';
COMMENT ON COLUMN group_invitations.invitee_id IS '被邀请人ID';
COMMENT ON COLUMN group_invitations.status IS '邀请状态：待处理、已接受、已拒绝';
COMMENT ON COLUMN group_invitations.created_at IS '创建时间';
COMMENT ON COLUMN group_invitations.updated_at IS '更新时间';

CREATE INDEX idx_group_invitations_group_id ON group_invitations (group_id);
CREATE INDEX idx_group_invitations_invitee_id ON group_invitations (invitee_id);
CREATE INDEX idx_group_invitations_status ON group_invitations (status);

CREATE TRIGGER update_group_invitations_modtime
    BEFORE UPDATE
    ON group_invitations
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();

-- 群二维码表
CREATE TABLE group_qrcodes
(
    id           VARCHAR(36) PRIMARY KEY,                            -- 二维码ID
    group_id     VARCHAR(36)  NOT NULL,                              -- 群组ID
    creator_id   VARCHAR(36)  NOT NULL,                              -- 创建者ID
    qrcode_url   VARCHAR(255) NOT NULL,                              -- 二维码URL
    expires_at   TIMESTAMP,                                          -- 过期时间，NULL表示永不过期
    is_permanent INTEGER      NOT NULL DEFAULT 0,                -- 是否永久有效，1-是，0-否
    created_at   TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP     -- 创建时间
);
COMMENT ON TABLE group_qrcodes IS '群二维码表';
COMMENT ON COLUMN group_qrcodes.id IS '二维码ID';
COMMENT ON COLUMN group_qrcodes.group_id IS '群组ID';
COMMENT ON COLUMN group_qrcodes.creator_id IS '创建者ID';
COMMENT ON COLUMN group_qrcodes.qrcode_url IS '二维码URL';
COMMENT ON COLUMN group_qrcodes.expires_at IS '过期时间，NULL表示永不过期';
COMMENT ON COLUMN group_qrcodes.is_permanent IS '是否永久有效，1-是，0-否';
COMMENT ON COLUMN group_qrcodes.created_at IS '创建时间';

CREATE INDEX idx_group_qrcodes_group_id ON group_qrcodes (group_id);

-- 群成员免打扰设置
CREATE TABLE group_member_settings
(
    id                 VARCHAR(36) PRIMARY KEY,                      -- 设置ID
    group_id           VARCHAR(36) NOT NULL,                         -- 群组ID
    user_id            VARCHAR(36) NOT NULL,                         -- 用户ID
    mute_notifications INTEGER     NOT NULL DEFAULT 0,           -- 是否免打扰，1-是，0-否
    nickname_in_group  VARCHAR(50),                                  -- 在群内的昵称
    created_at         TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP, -- 创建时间
    updated_at         TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP, -- 更新时间
    CONSTRAINT unique_member_settings UNIQUE (group_id, user_id)
);
COMMENT ON TABLE group_member_settings IS '群成员设置表';
COMMENT ON COLUMN group_member_settings.id IS '设置ID';
COMMENT ON COLUMN group_member_settings.group_id IS '群组ID';
COMMENT ON COLUMN group_member_settings.user_id IS '用户ID';
COMMENT ON COLUMN group_member_settings.mute_notifications IS '是否免打扰，1-是，0-否';
COMMENT ON COLUMN group_member_settings.nickname_in_group IS '在群内的昵称';
COMMENT ON COLUMN group_member_settings.created_at IS '创建时间';
COMMENT ON COLUMN group_member_settings.updated_at IS '更新时间';

CREATE INDEX idx_group_member_settings_group_id ON group_member_settings (group_id);
CREATE INDEX idx_group_member_settings_user_id ON group_member_settings (user_id);
CREATE INDEX idx_group_member_settings_user_group ON group_member_settings(user_id, group_id);

CREATE TRIGGER update_group_member_settings_modtime
    BEFORE UPDATE
    ON group_member_settings
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();