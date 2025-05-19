use crate::model::user::{CreateUserData, ForgetPasswordData, RegisterUserData, UpdateUserData, User};
use chrono::{TimeZone, Utc};
use common::utils::{hash_password, verify_password};
use common::{Error, Result};
use sqlx::{PgPool, QueryBuilder, Row};
use tracing::{debug, error};
use tracing::log::info;
use uuid::Uuid;

/// 用户仓库实现
pub struct UserRepository {
    pool: PgPool,
}

impl UserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 用户注册
    pub async fn register_user(&self, data: RegisterUserData) -> Result<User> {
        if data.tenant_id.is_empty() {
            // 检查企业号
            return Err(Error::BadRequest("企业号不能为空".to_string()));
        }
        // 用户名不为空
        if !data.username.is_empty() {
            // 检查用户名是否已存在
            if self.get_user_by_username(&data.username).await.is_ok() {
                return Err(Error::BadRequest(format!("用户名 {} 已被使用", data.username)));
            }
        }
        // 手机号不为空
        if !data.phone.is_empty() {
            // 检查手机号是否已存在
            if self.get_user_by_phone(&data.phone).await.is_ok() {
                return Err(Error::BadRequest(format!("手机号 {} 已被使用", data.phone)));
            }
        }
        // 生成密码哈希
        let password_hash = hash_password(&data.password)?;
        // 生成用户ID
        let id = Uuid::new_v4().simple();
        // 插入用户数据
        let row = sqlx::query!(
            r#"
            INSERT INTO users (id, username, password, phone, tenant_id)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            "#,
            id.to_string(),
            data.username,
            password_hash,
            data.phone,
            data.tenant_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            error!("用户注册失败: {}", err);
            Error::Database(err)
        })?;

        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };
        debug!("用户注册成功: {}", user.id);
        Ok(user)
    }

    /// 忘记密码 => 修改密码
    pub async fn forget_password(&self, data: ForgetPasswordData) -> Result<User> {
        // 检查企业号
        if data.tenant_id.is_empty() {
            return Err(Error::BadRequest("企业号不能为空".to_string()));
        }
        // 用户名或者手机号
        if data.username.is_empty() && data.phone.is_empty() {
            return Err(Error::BadRequest("用户名或者手机号不能为空".to_string()));
        }
        // 生成密码哈希
        let password_hash = hash_password(&data.password)?;
        // 插入用户数据
        let row = sqlx::query!(
            r#"
            UPDATE users
            SET password = COALESCE($1, password)
            WHERE username = $2 or phone = $3
            RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            "#,
            password_hash,
            data.username,
            data.phone
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            error!("修改密码失败: {}", err);
            Error::Database(err)
        })?;

        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };
        debug!("修改密码成功: {}", user.username);
        Ok(user)
    }

    /// 创建新用户
    pub async fn create_user(&self, data: CreateUserData) -> Result<User> {
        // 检查用户名是否已存在
        if self.get_user_by_username(&data.username).await.is_ok() {
            return Err(Error::BadRequest(format!(
                "用户名 {} 已被使用",
                data.username
            )));
        }

        // 检查邮箱是否已存在
        if self.get_user_by_email(&data.email).await.is_ok() {
            return Err(Error::BadRequest(format!("邮箱 {} 已被使用", data.email)));
        }

        // 生成密码哈希
        let password_hash = hash_password(&data.password)?;

        // 生成用户ID
        let id = Uuid::new_v4().simple();

        // 插入用户数据
        let row = sqlx::query!(
            r#"
            INSERT INTO users (id, username, email, password, nickname, avatar_url)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            "#,
            id.to_string(),
            data.username,
            data.email,
            password_hash,
            data.nickname,
            data.avatar_url
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            error!("创建用户失败: {}", err);
            Error::Database(err)
        })?;

        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };

        debug!("用户创建成功: {}", user.id);
        Ok(user)
    }

    /// 根据ID查询用户
    pub async fn get_user_by_id(&self, id: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            FROM users
            WHERE id = $1
            "#,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = err {
                Error::NotFound(format!("用户ID {} 不存在", id))
            } else {
                error!("查询用户失败: {}", err);
                Error::Database(err)
            }
        })?;

        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };

        Ok(user)
    }

    /// 根据用户名查询用户
    pub async fn get_user_by_username(&self, username: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            FROM users
            WHERE username = $1
            "#,
            username
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = err {
                Error::NotFound(format!("用户名 {} 不存在", username))
            } else {
                error!("查询用户失败: {}", err);
                Error::Database(err)
            }
        })?;

        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };

        Ok(user)
    }

    /// 根据邮箱查询用户
    pub async fn get_user_by_email(&self, email: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            FROM users
            WHERE email = $1
            "#,
            email
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = err {
                Error::NotFound(format!("邮箱 {} 不存在", email))
            } else {
                error!("查询用户失败: {}", err);
                Error::Database(err)
            }
        })?;

        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.updated_at,
            user_idx: row.user_idx,
        };

        Ok(user)
    }

    /// 根据手机号查询用户
    pub async fn get_user_by_phone(&self, phone: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            FROM users
            WHERE phone = $1
            "#,
            phone
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = err {
                Error::NotFound(format!("手机号 {} 不存在", phone))
            } else {
                error!("查询用户失败: {}", err);
                Error::Database(err)
            }
        })?;
        let user = User {
            id: row.id,
            username: row.username.unwrap_or_default(),
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone.unwrap_or_default(),
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };
        Ok(user)
    }

    /// 更新用户信息
    pub async fn update_user(&self, id: &str, data: UpdateUserData) -> Result<User> {

        // 检查用户是否存在
        let _user = self.get_user_by_id(id).await?;

        // 更新密码，如果有提供的话
        let password_hash = if let Some(password) = &data.password {
            Some(hash_password(password)?)
        } else {
            None
        };

        // 动态构建SET子句
        let mut builder = QueryBuilder::new(" UPDATE users SET ");
        let mut first = true;
        if let Some(email) = data.email {
            if !first { builder.push(","); }
            builder.push(" email = COALESCE(" ).push_bind(email).push(", email) ");
            first = false;
        }
        if let Some(nickname) = data.nickname {
            if !first { builder.push(","); }
            builder.push(" nickname = COALESCE( ").push_bind(nickname).push(", nickname) ");
            first = false;
        }
        if let Some(head_image) = data.head_image {
            if !first { builder.push(","); }
            builder.push(" head_image = COALESCE( ").push_bind(head_image).push(", head_image) ");
            first = false;
        }
        if let Some(head_image_thumb) = data.head_image_thumb {
            if !first { builder.push(","); }
            builder.push(" head_image_thumb = COALESCE( ").push_bind(head_image_thumb).push(", head_image) ");
            first = false;
        }
        if let Some(sex) = data.sex {
            if !first { builder.push(","); }
            builder.push(" sex = COALESCE( ").push_bind(sex.to_string()).push(", sex) ");
            first = false;
        }
        if let Some(password) = data.password {
            if !first { builder.push(","); }
            builder.push(" password = COALESCE( ").push_bind(hash_password(&password)?).push(", password) ");
            first = false;
        }

        if !first { builder.push(","); }
        builder.push(" updated_at = ").push_bind(Utc::now());
        builder.push(" WHERE id = ").push_bind(&data.user_id);
        builder.push(" RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, user_idx "
        );
        // 生成最终SQL
        let query = builder.build_query_as::<User>();
        let row = query.fetch_one(&self.pool).await?;

        let updated_user = User {
            id: row.id,
            username: row.username,
            email: row.email,
            password: row.password,
            nickname: row.nickname,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex,
            user_stat: row.user_stat,
            tenant_id: row.tenant_id,
            last_login_time: row.last_login_time,
            user_idx: row.user_idx,
        };

        debug!("用户更新成功: {}", updated_user.id);
        Ok(updated_user)
    }

    /// 验证用户密码
    pub async fn verify_user_password(&self, username: &str, password: &str) -> Result<User> {
        // 查询用户
        let user = self.get_user_by_username(username).await?;

        // 验证密码
        let is_valid = verify_password(password, &user.password)?;

        if !is_valid {
            return Err(Error::Authentication("密码不正确".to_string()));
        }

        Ok(user)
    }

    /// 搜索用户
    pub async fn search_users(
        &self,
        query: &str,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<User>, i32)> {
        // 计算分页
        let offset = (page - 1) * page_size;

        // 构造搜索条件
        let search_pattern = format!("%{}%", query);

        // 查询符合条件的用户
        let rows = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time,
            user_idx
            FROM users
            WHERE username ILIKE $1 OR email ILIKE $1 OR COALESCE(nickname, '') ILIKE $1
            ORDER BY username
            LIMIT $2 OFFSET $3
            "#,
            search_pattern,
            page_size as i64,
            offset as i64
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| {
            error!("搜索用户失败: {}", err);
            Error::Database(err)
        })?;

        let users = rows
            .into_iter()
            .map(|row| User {
                id: row.id,
                username: row.username.unwrap_or_default(),
                email: row.email,
                password: row.password,
                nickname: row.nickname,
                avatar_url: row.avatar_url,
                created_at: row.created_at,
                updated_at: row.updated_at,
                phone: row.phone.unwrap_or_default(),
                address: row.address,
                head_image: row.head_image,
                head_image_thumb: row.head_image_thumb,
                sex: row.sex.map(|x| x as i32),
                user_stat: row.user_stat.unwrap_or_default() as i32,
                tenant_id: row.tenant_id.unwrap_or_default(),
                last_login_time: row.last_login_time,
                user_idx: row.user_idx
            })
            .collect();

        // 查询总数
        let total: i64 = sqlx::query(
            r#"
            SELECT COUNT(*) as total
            FROM users
            WHERE username ILIKE $1 OR email ILIKE $1 OR COALESCE(nickname, '') ILIKE $1
            "#,
        )
        .bind(&search_pattern)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            error!("查询用户总数失败: {}", err);
            Error::Database(err)
        })?
        .get("total");

        Ok((users, total as i32))
    }
}
