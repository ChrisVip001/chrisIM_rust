-- 在用户表中添加个性签名字段
ALTER TABLE users ADD COLUMN sign VARCHAR(100);

-- 添加字段注释
COMMENT ON COLUMN users.sign IS '用户个性签名';

-- 为user_config表添加是否展示手机号字段
ALTER TABLE user_config ADD COLUMN show_phone int4 DEFAULT 2;

-- 添加字段注释
COMMENT ON COLUMN user_config.show_phone IS '是否展示手机号(1-是，2-否)';
