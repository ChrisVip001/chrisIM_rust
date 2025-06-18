ALTER TABLE user_config ALTER COLUMN allow_phone_search DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN allow_phone_search SET DEFAULT 1;

ALTER TABLE user_config ALTER COLUMN allow_id_search DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN allow_id_search SET DEFAULT 1;

ALTER TABLE user_config ALTER COLUMN auto_load_video DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN auto_load_video SET DEFAULT 2;

ALTER TABLE user_config ALTER COLUMN auto_load_pic DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN auto_load_pic SET DEFAULT 2;

ALTER TABLE user_config ALTER COLUMN msg_read_flag DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN msg_read_flag SET DEFAULT 1;

ALTER TABLE user_config ALTER COLUMN sound_enabled DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN sound_enabled SET DEFAULT 1;

ALTER TABLE user_config ALTER COLUMN vibration_enabled DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN vibration_enabled SET DEFAULT 2;

ALTER TABLE user_config ALTER COLUMN show_phone DROP DEFAULT;
ALTER TABLE user_config ALTER COLUMN show_phone SET DEFAULT 2;
