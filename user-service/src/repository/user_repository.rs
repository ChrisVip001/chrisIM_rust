use crate::model::user::{CreateUserData, ForgetPasswordData, RegisterUserData, UpdateUserData, User};
use chrono::{TimeZone, Utc};
use common::utils::{generate_user_id, hash_password, verify_password};
use common::{Error, Result};
use sqlx::{PgPool, QueryBuilder, Row};
use tonic::Status;
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

    /// 检查自定义ID是否已存在
    pub async fn is_custom_id_exists(&self, custom_id: &str) -> Result<bool> {
        let result = sqlx::query!(
            "SELECT id FROM users WHERE custom_id = $1",
            custom_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| {
            error!("检查自定义ID失败: {}", err);
            Error::Database(err)
        })?;
        
        Ok(result.is_some())
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
        let mut password_hash = hash_password("A123456")?;
        if !data.password.clone().is_empty(){
            password_hash = hash_password(&data.password)?;
        }
        // 生成用户ID
        let id = generate_user_id()
            .map_err(|e| Error::Internal(format!("生成用户ID失败: {}", e)))?;
        // 插入用户数据
        let row = sqlx::query!(
            r#"
            INSERT INTO users (id, username, password, phone, tenant_id, custom_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id,sign
            "#,
            id.to_string(),
            data.username.clone(),
            password_hash,
            data.phone,
            data.tenant_id,
            data.custom_id
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
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
        let phone = data.phone;
        let user=match self.get_user_by_phone(&phone).await {
            Ok(u) => {u}, // 用户存在，继续处理
            Err(err) => {
                error!("手机号对应用户不存在: {}, 错误: {}", phone, err);
                return Err(Error::BadRequest("手机号对应用户不存在".to_string()));
            }
        };        
        // 生成密码哈希
        let password_hash = hash_password(&data.password)?;
        // 插入用户数据
        let row = sqlx::query!(
            r#"
            UPDATE users
            SET password = $1
            WHERE id = $2 
            RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id ,sign
            "#,
            password_hash,
            user.id
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
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
        let mut password_hash = hash_password("A123456")?;
        if !data.password.clone().is_empty(){
            password_hash = hash_password(&data.password)?;
        }
        // 生成用户ID
        let id = generate_user_id()
            .map_err(|e| Error::Internal(format!("生成用户ID失败: {}", e)))?;
        // 插入用户数据
        let row = sqlx::query!(
            r#"
            INSERT INTO users (id, username, email, password, nickname, avatar_url, custom_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id ,sign
            "#,
            id.to_string(),
            data.username,
            data.email,
            password_hash,
            data.nickname,
            data.avatar_url,
            data.custom_id
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
        };

        debug!("用户创建成功: {}", user.id);
        Ok(user)
    }

    /// 根据ID查询用户
    pub async fn get_user_by_id(&self, id: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
            FROM users
            WHERE id = $1
            "#,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = &err {
                Error::NotFound(format!("用户未找到: {}", id))
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex,
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
        };
        Ok(user)
    }

    /// 根据用户名查询用户
    pub async fn get_user_by_username(&self, username: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
            FROM users
            WHERE username = $1
            "#,
            username
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = &err {
                Error::NotFound(format!("用户未找到: {}", username))
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex,
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
        };
        Ok(user)
    }

    /// 根据用户名查询用户
    pub async fn get_user_by_custom_id(&self, custom_id: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
            FROM users
            WHERE custom_id = $1
            "#,
            custom_id
        )
            .fetch_one(&self.pool)
            .await
            .map_err(|err| {
                if let sqlx::Error::RowNotFound = &err {
                    Error::NotFound(format!("用户未找到: {}", custom_id))
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex,
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
        };
        Ok(user)
    }
    
    /// 根据用户名或手机号查询用户
    pub async fn get_user_by_username_phone(&self, username: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
            FROM users
            WHERE username = $1 or phone =$1
            "#,
            username
        )
            .fetch_one(&self.pool)
            .await
            .map_err(|err| {
                if let sqlx::Error::RowNotFound = err {
                    Error::NotFound(format!("用户名或手机号 {} 不存在", username))
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
        };

        Ok(user)
    }

    /// 根据邮箱查询用户
    pub async fn get_user_by_email(&self, email: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.updated_at,
            custom_id: row.custom_id,
            sign: row.sign,
        };

        Ok(user)
    }

    /// 根据手机号查询用户
    pub async fn get_user_by_phone(&self, phone: &str) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
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
            phone: row.phone,
            address: row.address,
            head_image: row.head_image,
            head_image_thumb: row.head_image_thumb,
            sex: row.sex.map(|x| x as i32),
            user_stat: row.user_stat.unwrap_or_default() as i32,
            tenant_id: row.tenant_id.unwrap_or_default(),
            last_login_time: row.last_login_time,
            custom_id: row.custom_id,
            sign: row.sign,
        };
        Ok(user)
    }

    /// 更新用户信息
    pub async fn update_user(&self, id: &str, data: UpdateUserData) -> Result<User> {

        // 检查用户是否存在
        let _user = self.get_user_by_id(id).await?;

        // 动态构建SET子句
        let mut builder = QueryBuilder::new(" UPDATE users SET ");
        let mut first = true;
        if let Some(username) = data.username {
            // 用户名做唯一校验
            if let Ok(existing_user) = self.get_user_by_username(&username).await {
                if existing_user.id != id {
                    return Err(Error::BadRequest(format!("custom_id {} 已被使用", username)));
                }
            }
            
            if !first { builder.push(","); }
            builder.push(" username = COALESCE(" ).push_bind(username).push(", username) ");
            first = false;
        }
        if let Some(user_id) = data.user_id {
            if !first { builder.push(","); }
            builder.push(" id = COALESCE(" ).push_bind(user_id).push(", id) ");
            first = false;
        }
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
            builder.push(" sex = COALESCE( ").push_bind(sex as i32).push(", sex) ");
            first = false;
        }
        if let Some(password) = data.password {
            if !first { builder.push(","); }
            builder.push(" password = COALESCE( ").push_bind(hash_password(&password)?).push(", password) ");
            first = false;
        }
        if let Some(custom_id) = data.custom_id {
            // custom_id做唯一校验
            // 用户名做唯一校验
            if let Ok(existing_user) = self.get_user_by_custom_id(&custom_id).await {
                if existing_user.id != id {
                    return Err(Error::BadRequest(format!("custom_id {} 已被使用", custom_id)));
                }
            }
            
            if !first { builder.push(","); }
            builder.push(" custom_id = COALESCE( ").push_bind(custom_id).push(", custom_id) ");
            first = false;
        }
        if let Some(address) = data.address {
            if !first { builder.push(","); }
            builder.push(" address = COALESCE( ").push_bind(address).push(", custom_id) ");
            first = false;
        }
        if let Some(sign) = data.sign {
            if !first { builder.push(","); }
            builder.push(" sign = COALESCE( ").push_bind(sign).push(", sign) ");
            first = false;
        }

        if let Some(avatar_url) = data.avatar_url {
            if !first { builder.push(","); }
            builder.push(" avatar_url = COALESCE( ").push_bind(avatar_url).push(", avatar_url) ");
            first = false;
        }

        if !first { builder.push(","); }
        builder.push(" updated_at = ").push_bind(Utc::now());
        builder.push(" WHERE id = ").push_bind(id);
        builder.push(" RETURNING id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign "
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
            custom_id: row.custom_id,
            sign: row.sign,
        };

        debug!("用户更新成功: {}", updated_user.id);
        Ok(updated_user)
    }

    /// 验证用户密码(用户名或手机号验证)
    pub async fn verify_user_password(&self, username: &str, password: &str) -> Result<User> {
        // 查询用户
        let user = self.get_user_by_username_phone(username).await?;

        // 验证密码
        let is_valid = verify_password(password, &user.password)?;

        if !is_valid {
            return Err(Error::Authentication("密码不正确".to_string()));
        }

        Ok(user)
    }

    /// 验证用户密码(id验证)
    pub async fn verify_user_password_by_id(&self, username: &str, password: &str) -> Result<bool> {
        // 查询用户
        let user = self.get_user_by_username_phone(username).await?;

        // 验证密码
        let is_valid = verify_password(password, &user.password)?;

        if !is_valid {
            return Err(Error::Authentication("密码不正确".to_string()));
        }

        Ok(true)
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
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
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
                phone: row.phone,
                address: row.address,
                head_image: row.head_image,
                head_image_thumb: row.head_image_thumb,
                sex: row.sex.map(|x| x as i32),
                user_stat: row.user_stat.unwrap_or_default() as i32,
                tenant_id: row.tenant_id.unwrap_or_default(),
                last_login_time: row.last_login_time,
                custom_id: row.custom_id,
                sign: row.sign,
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

    /// 注销用户账号（软删除+匿名化）
    pub async fn deactivate_user(&self, user_id: &str) -> Result<bool> {
        // 获取用户信息
        let _user = self.get_user_by_id(user_id).await?;
        
        // 生成匿名用户名和随机密码
        let anon_username = format!("deactivated_{}", Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>());
        let anon_password = hash_password(&Uuid::new_v4().to_string())?;
        
        // 为手机号生成随机值（保留前缀，确保唯一性）
        let anon_phone = format!("deact{}", Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>());
        
        // 执行用户注销操作 - 软删除和匿名化处理
        // 1. 将用户状态修改为已注销(3)
        // 2. 匿名化用户敏感信息
        let result = sqlx::query!(
            r#"
            UPDATE users
            SET 
                username = $1,
                email = NULL,
                password = $2,
                nickname = '已注销用户',
                avatar_url = NULL,
                phone = $3,
                address = NULL,
                head_image = NULL,
                head_image_thumb = NULL,
                user_stat = 3,
                updated_at = NOW()
            WHERE id = $4
            "#,
            anon_username,
            anon_password,
            anon_phone,
            user_id
        )
        .execute(&self.pool)
        .await
        .map_err(|err| {
            error!("注销用户失败: {}", err);
            Error::Database(err)
        })?;
        
        debug!("成功注销用户: {}, 影响行数: {}", user_id, result.rows_affected());
        Ok(result.rows_affected() > 0)
    }

    /// 更新用户手机号
    pub async fn update_phone(&self, user_id: &str, new_phone: &str) -> Result<User> {
        // 检查用户是否存在
        let _user = self.get_user_by_id(user_id).await?;
        // 执行更新
        sqlx::query!(
            r#"
            UPDATE users
            SET phone = $1, updated_at = NOW()
            WHERE id = $2
            "#,
            new_phone,
            user_id
        )
            .execute(&self.pool)
            .await
            .map_err(|err| {
                error!("更新用户手机号失败: {}", err);
                Error::Database(err)
            })?;
        // 获取更新后的用户信息
        self.get_user_by_id(user_id).await
    }

    /// 根据用户ID列表批量获取用户
    pub async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>> {
        if user_ids.is_empty() {
            return Ok(Vec::new());
        }

        // 构建SQL查询
        let query = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password, nickname, avatar_url, created_at, updated_at,
            phone, address, head_image, head_image_thumb, sex, user_stat, tenant_id, last_login_time, custom_id, sign
            FROM users
            WHERE id = ANY($1)
            "#
        )
        .bind(user_ids);

        // 执行查询
        let users = query.fetch_all(&self.pool).await.map_err(|e| {
            error!("批量获取用户失败: {}", e);
            Error::Database(e)
        })?;

        Ok(users)
    }
}
