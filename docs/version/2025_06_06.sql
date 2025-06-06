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