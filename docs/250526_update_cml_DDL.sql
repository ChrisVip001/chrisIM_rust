ALTER TABLE users
    ADD COLUMN custom_id VARCHAR(20) NOT NULL
        CONSTRAINT unique_custom_id unique;

COMMENT
ON COLUMN users.custom_id
IS '用户自定义ID';
