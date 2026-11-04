drop table if exists users;
CREATE TABLE users
(
    "id"               varchar(36)  NOT NULL,
    "username"         varchar(36),
    "email"            varchar(100),
    "password"         varchar(128) NOT NULL,
    "nickname"         varchar(50),
    "avatar_url"       varchar(255),
    "created_at"       timestamp(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at"       timestamp(6),
    "phone"            varchar(36)  NOT NULL,
    "address"          varchar(255),
    "head_image"       varchar(255),
    "head_image_thumb" varchar(255),
    "sex"              int2,
    "user_stat"        int2                  DEFAULT 1,
    "tenant_id"        varchar(20),
    "last_login_time"  timestamp(6),
    "user_idx"         varchar(20),
    CONSTRAINT "users_pkey" PRIMARY KEY ("id"),
    CONSTRAINT "idx_email" UNIQUE ("email"),
    CONSTRAINT "idx_username" UNIQUE ("username")
);
COMMENT ON COLUMN "public"."users"."id" IS '用户ID';
COMMENT ON COLUMN "public"."users"."username" IS '用户名';
COMMENT ON COLUMN "public"."users"."email" IS '邮箱';
COMMENT ON COLUMN "public"."users"."password" IS '密码';
COMMENT ON COLUMN "public"."users"."nickname" IS '昵称';
COMMENT ON COLUMN "public"."users"."created_at" IS '创建时间';
COMMENT ON COLUMN "public"."users"."updated_at" IS '修改时间';
COMMENT ON COLUMN "public"."users"."phone" IS '手机号';
COMMENT ON COLUMN "public"."users"."address" IS '地址';
COMMENT ON COLUMN "public"."users"."head_image" IS '用户头像';
COMMENT ON COLUMN "public"."users"."head_image_thumb" IS '用户头像缩略图';
COMMENT ON COLUMN "public"."users"."sex" IS '性别 1:男 2:女';
COMMENT ON COLUMN "public"."users"."user_stat" IS '用户状态(1-正常，2-已删除 3-已注销)';
COMMENT ON COLUMN "public"."users"."tenant_id" IS '企业号';
COMMENT ON COLUMN "public"."users"."last_login_time" IS '最后登录时间';
COMMENT ON COLUMN "public"."users"."user_idx" IS '用户唯一ID';
COMMENT ON TABLE "public"."users" IS '用户表';
