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
    CONSTRAINT unique_friendship UNIQUE (user_id, friend_id)
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

-- 群组表
CREATE TABLE groups
(
    id          VARCHAR(36) PRIMARY KEY,
    name        VARCHAR(100) NOT NULL,
    description TEXT,
    avatar_url  VARCHAR(255),
    owner_id    VARCHAR(36)  NOT NULL,
    created_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP
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
    CONSTRAINT unique_membership UNIQUE (group_id, user_id)
);

CREATE INDEX idx_group_members_group_id ON group_members (group_id);
CREATE INDEX idx_group_members_user_id ON group_members (user_id);

CREATE TABLE messages
(
    send_id      VARCHAR NOT NULL,
    receiver_id  VARCHAR NOT NULL,
    local_id     VARCHAR NOT NULL,
    server_id    VARCHAR NOT NULL,
    send_time    BIGINT  NOT NULL,
    msg_type     INT,
    content_type INT,
    content      BYTEA,
    platform     INT,
    PRIMARY KEY (send_id, server_id, send_time)
) PARTITION BY RANGE (send_time);

DO $$
    DECLARE
        week_start_ms BIGINT;
        week_end_ms BIGINT;
        current_week_start TIMESTAMP;
        year_week_num VARCHAR(10);
    BEGIN
        -- 获取当前时间戳所在周的开始时间（使用毫秒）
        current_week_start := DATE_TRUNC('week', NOW());
        FOR i IN 0..51 LOOP
                -- 计算每个分区的开始和结束时间戳（毫秒）
                week_start_ms := (EXTRACT(EPOCH FROM current_week_start + (i * INTERVAL '1 week')) * 1000)::BIGINT;
                week_end_ms := (EXTRACT(EPOCH FROM current_week_start + ((i + 1) * INTERVAL '1 week')) * 1000)::BIGINT;
                -- 年份和周数标识
                year_week_num := TO_CHAR(current_week_start + (i * INTERVAL '1 week'), 'IYYY_IW');

                -- 创建分区表
                EXECUTE 'CREATE TABLE IF NOT EXISTS messages_' || year_week_num || ' PARTITION OF messages FOR VALUES FROM (' || week_start_ms || ') TO (' || week_end_ms || ')';
            END LOOP;
    END
$$;