-- 用户表 (PostgreSQL兼容版本)
CREATE TABLE users
(
    id         VARCHAR(36) PRIMARY KEY,
    username   VARCHAR(50)  NOT NULL UNIQUE,
    email      VARCHAR(100) NOT NULL UNIQUE,
    password   VARCHAR(128) NOT NULL,
    nickname   VARCHAR(50),
    avatar_url VARCHAR(255),
    created_at TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT idx_username UNIQUE (username),
    CONSTRAINT idx_email UNIQUE (email)
);

-- 创建一个触发器来自动更新updated_at字段
CREATE
OR REPLACE FUNCTION update_modified_column()
    RETURNS TRIGGER AS
$$
BEGIN
    NEW.updated_at
= CURRENT_TIMESTAMP;
RETURN NEW;
END;
$$
LANGUAGE plpgsql;

CREATE TRIGGER update_users_modtime
    BEFORE UPDATE
    ON users
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();

-- 好友关系表
CREATE TABLE friendships
(
    id         VARCHAR(36) PRIMARY KEY,
    user_id    VARCHAR(36) NOT NULL,
    friend_id  VARCHAR(36) NOT NULL,
    status     VARCHAR(10) NOT NULL DEFAULT 'PENDING',
    created_at TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT check_status CHECK (status IN ('PENDING', 'ACCEPTED', 'REJECTED', 'BLOCKED')),
    CONSTRAINT unique_friendship UNIQUE (user_id, friend_id),
    CONSTRAINT fk_user_id FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT fk_friend_id FOREIGN KEY (friend_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX idx_friendships_user_id ON friendships (user_id);
CREATE INDEX idx_friendships_friend_id ON friendships (friend_id);
CREATE INDEX idx_friendships_status ON friendships (status);

-- 创建触发器自动更新updated_at
CREATE TRIGGER update_friendships_modtime
    BEFORE UPDATE
    ON friendships
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();


-- 私聊消息表
CREATE TABLE private_messages
(
    id           VARCHAR(36) PRIMARY KEY,
    sender_id    VARCHAR(36) NOT NULL,
    receiver_id  VARCHAR(36) NOT NULL,
    content      TEXT        NOT NULL,
    content_type VARCHAR(10) NOT NULL DEFAULT 'TEXT',
    sent_at      TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    is_read      BOOLEAN     NOT NULL DEFAULT FALSE,
    is_deleted   BOOLEAN     NOT NULL DEFAULT FALSE,
    CONSTRAINT check_content_type CHECK (content_type IN ('TEXT', 'IMAGE', 'AUDIO', 'VIDEO', 'FILE')),
    CONSTRAINT fk_sender_id FOREIGN KEY (sender_id) REFERENCES users (id) ON DELETE SET NULL,
    CONSTRAINT fk_receiver_id FOREIGN KEY (receiver_id) REFERENCES users (id) ON DELETE SET NULL
);

CREATE INDEX idx_private_messages_sender_id ON private_messages (sender_id);
CREATE INDEX idx_private_messages_receiver_id ON private_messages (receiver_id);
CREATE INDEX idx_private_messages_sent_at ON private_messages (sent_at);


-- 群组表
CREATE TABLE groups
(
    id          VARCHAR(36) PRIMARY KEY,
    name        VARCHAR(100) NOT NULL,
    description TEXT,
    avatar_url  VARCHAR(255),
    owner_id    VARCHAR(36)  NOT NULL,
    created_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT fk_owner_id FOREIGN KEY (owner_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX idx_groups_owner_id ON groups (owner_id);
CREATE INDEX idx_groups_name ON groups (name);

-- 创建触发器自动更新updated_at
CREATE TRIGGER update_groups_modtime
    BEFORE UPDATE
    ON groups
    FOR EACH ROW
    EXECUTE FUNCTION update_modified_column();

-- 群组成员表
CREATE TABLE group_members
(
    id        VARCHAR(36) PRIMARY KEY,
    group_id  VARCHAR(36) NOT NULL,
    user_id   VARCHAR(36) NOT NULL,
    role      VARCHAR(10) NOT NULL DEFAULT 'MEMBER',
    joined_at TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT check_role CHECK (role IN ('MEMBER', 'ADMIN', 'OWNER')),
    CONSTRAINT unique_membership UNIQUE (group_id, user_id),
    CONSTRAINT fk_group_id FOREIGN KEY (group_id) REFERENCES groups (id) ON DELETE CASCADE,
    CONSTRAINT fk_user_id FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE INDEX idx_group_members_group_id ON group_members (group_id);
CREATE INDEX idx_group_members_user_id ON group_members (user_id);

-- 群组消息表
CREATE TABLE group_messages
(
    id           VARCHAR(36) PRIMARY KEY,
    group_id     VARCHAR(36) NOT NULL,
    sender_id    VARCHAR(36) NOT NULL,
    content      TEXT        NOT NULL,
    content_type VARCHAR(10) NOT NULL DEFAULT 'TEXT',
    sent_at      TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    is_deleted   BOOLEAN     NOT NULL DEFAULT FALSE,
    CONSTRAINT check_content_type CHECK (content_type IN ('TEXT', 'IMAGE', 'AUDIO', 'VIDEO', 'FILE')),
    CONSTRAINT fk_group_id FOREIGN KEY (group_id) REFERENCES groups (id) ON DELETE CASCADE,
    CONSTRAINT fk_sender_id FOREIGN KEY (sender_id) REFERENCES users (id) ON DELETE SET NULL
);

CREATE INDEX idx_group_messages_group_id ON group_messages (group_id);
CREATE INDEX idx_group_messages_sender_id ON group_messages (sender_id);
CREATE INDEX idx_group_messages_sent_at ON group_messages (sent_at);