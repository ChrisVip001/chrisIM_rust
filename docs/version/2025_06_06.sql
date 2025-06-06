-- 修改群成员设置表，添加新字段
ALTER TABLE group_member_settings
ADD COLUMN remark VARCHAR(255), -- 群备注
ADD COLUMN is_top INTEGER NOT NULL DEFAULT 0, -- 群置顶，1-是，0-否
ADD COLUMN recall_notification INTEGER NOT NULL DEFAULT 0; -- 群撤回通知，1-是，0-否
ADD COLUMN show_nickname INTEGER NOT NULL DEFAULT 1; -- 是否显示群昵称，1-是，0-否

COMMENT ON COLUMN group_member_settings.remark IS '群备注';
COMMENT ON COLUMN group_member_settings.is_top IS '群置顶，1-是，0-否';
COMMENT ON COLUMN group_member_settings.recall_notification IS '群撤回通知，1-是，0-否';
COMMENT ON COLUMN group_member_settings.show_nickname IS '是否显示群昵称，1-是，0-否';

-- 为群组设置表增加新字段
ALTER TABLE group_settings ADD COLUMN notify_member_join INTEGER NOT NULL DEFAULT 1; -- 是否开启群成员进群提醒，1-开启，0-关闭
ALTER TABLE group_settings ADD COLUMN all_member_muted INTEGER NOT NULL DEFAULT 0; -- 是否开启全员禁言，1-开启，0-关闭

-- 添加字段注释
COMMENT ON COLUMN group_settings.notify_member_join IS '是否开启群成员进群提醒，1-开启，0-关闭';
COMMENT ON COLUMN group_settings.all_member_muted IS '是否开启全员禁言，1-开启，0-关闭';