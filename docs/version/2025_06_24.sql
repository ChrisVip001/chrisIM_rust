ALTER TABLE group_members
ADD COLUMN is_muted integer NOT NULL DEFAULT 0; -- 群成员禁言状态 (0-未禁言，1-已禁言)

COMMENT ON COLUMN group_members.is_muted IS '群成员禁言状态 (0-未禁言，1-已禁言)';

ALTER TABLE groups
ADD COLUMN all_member_muted integer NOT NULL DEFAULT 0; -- 全员禁言状态 (0-未禁言，1-已禁言)

COMMENT ON COLUMN groups.all_member_muted IS '全员禁言状态 (0-未禁言，1-已禁言)';
