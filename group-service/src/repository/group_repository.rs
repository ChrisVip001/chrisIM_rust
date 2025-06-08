use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;

use crate::model::group::{Group, UserGroup};

pub struct GroupRepository {
    pool: PgPool,
}

impl GroupRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 创建群组
    pub async fn create_group(
        &self,
        name: String,
        description: String,
        avatar_url: String,
        owner_id: String,
    ) -> Result<Group> {
        let group = Group::new(name, description, avatar_url, owner_id);

        // 将DateTime<Utc>转换为NaiveDateTime
        let created_at_naive = group.created_at.naive_utc();
        let updated_at_naive = group.updated_at.naive_utc();

        let result = sqlx::query!(
            r#"
            INSERT INTO groups (id, name, description, avatar_url, owner_id, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, name, description, avatar_url, owner_id, created_at, updated_at
            "#,
            group.id,
            group.name,
            group.description,
            group.avatar_url,
            group.owner_id,
            created_at_naive,
            updated_at_naive
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Group {
            id: result.id,
            name: result.name,
            description: result.description.unwrap_or_default(),
            avatar_url: result.avatar_url.unwrap_or_default(),
            owner_id: result.owner_id,
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }

    // 获取群组信息
    pub async fn get_group(&self, group_id: String) -> Result<Group> {
        let result = sqlx::query!(
            r#"
            SELECT id, name, description, avatar_url, owner_id, created_at, updated_at
            FROM groups
            WHERE id = $1
            "#,
            group_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Group {
            id: result.id,
            name: result.name,
            description: result.description.unwrap_or_default(),
            avatar_url: result.avatar_url.unwrap_or_default(),
            owner_id: result.owner_id,
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }

    // 更新群组信息
    pub async fn update_group(
        &self,
        group_id: String,
        name: Option<String>,
        description: Option<String>,
        avatar_url: Option<String>,
    ) -> Result<Group> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 先获取现有数据
        let current = self.get_group(group_id.clone()).await?;

        // 更新群组信息
        let result = sqlx::query!(
            r#"
            UPDATE groups
            SET name = $1, description = $2, avatar_url = $3, updated_at = $4
            WHERE id = $5
            RETURNING id, name, description, avatar_url, owner_id, created_at, updated_at
            "#,
            name.unwrap_or(current.name),
            description.unwrap_or(current.description),
            avatar_url.unwrap_or(current.avatar_url),
            now_naive,
            group_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Group {
            id: result.id,
            name: result.name,
            description: result.description.unwrap_or_default(),
            avatar_url: result.avatar_url.unwrap_or_default(),
            owner_id: result.owner_id,
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }

    // 删除群组
    pub async fn delete_group(&self, group_id: String, user_id: String) -> Result<bool> {
        // 先检查是否是群主
        let group = self.get_group(group_id.clone()).await?;
        if group.owner_id != user_id {
            return Err(anyhow::anyhow!("只有群主可以删除群组"));
        }

        let rows_affected = sqlx::query!(
            r#"
            DELETE FROM groups
            WHERE id = $1
            "#,
            group_id
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }

    // 获取群组成员数量
    pub async fn get_member_count(&self, group_id: String) -> Result<i32> {
        let result = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM group_members
            WHERE group_id = $1
            "#,
            group_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result.count.unwrap_or(0) as i32)
    }

    // 获取用户加入的群组列表
    pub async fn get_user_groups(&self, user_id: String) -> Result<Vec<UserGroup>> {
        let groups = sqlx::query!(
            r#"
            SELECT 
                g.id,
                g.name,
                g.avatar_url,
                m.role,
                m.joined_at,
                (SELECT COUNT(*) FROM group_members WHERE group_id = g.id) as member_count
            FROM groups g
            JOIN group_members m ON g.id = m.group_id
            WHERE m.user_id = $1
            "#,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;

        let result = groups
            .into_iter()
            .map(|g| UserGroup {
                id: g.id,
                name: g.name,
                avatar_url: g.avatar_url.unwrap_or_default(),
                member_count: g.member_count.unwrap_or(0) as i32,
                role: g.role.parse::<i32>().unwrap_or(0),
                joined_at: Utc.from_utc_datetime(&g.joined_at),
                remark: String::new(), // 群备注默认为空，将在service层填充
            })
            .collect();

        Ok(result)
    }

    
    // 搜索用户加入的群组（按关键字）
    pub async fn search_user_groups(
        &self,
        user_id: String,
        keyword: Option<&str>,
        page: Option<i32>,
        page_size: Option<i32>,
    ) -> Result<(Vec<UserGroup>, i64)> {
        // 设置默认值
        // 默认分页参数
        let page = page.unwrap_or(1);
        let page_size = page_size.unwrap_or(20);
        
        // 计算偏移量
        let offset = (page - 1) * page_size;
        
        let mut result: Vec<UserGroup> = Vec::new();
        let total: i64;
        
        // 根据是否有关键字构建不同的查询
        if let Some(kw) = keyword {
            // 有关键字时的查询
            let groups = sqlx::query!(
                r#"
                SELECT 
                    g.id,
                    g.name,
                    g.avatar_url,
                    m.role,
                    m.joined_at,
                    (SELECT COUNT(*) FROM group_members WHERE group_id = g.id) as member_count
                FROM groups g
                JOIN group_members m ON g.id = m.group_id
                WHERE m.user_id = $1
                AND (g.name ILIKE $2 OR g.description ILIKE $2)
                ORDER BY g.name
                LIMIT $3 OFFSET $4
                "#,
                user_id,
                format!("%{}%", kw),
                page_size as i64,
                offset as i64
            )
            .fetch_all(&self.pool)
            .await?;

            // 将查询结果转换为UserGroup对象
            for g in groups {
                result.push(UserGroup {
                    id: g.id,
                    name: g.name,
                    avatar_url: g.avatar_url.unwrap_or_default(),
                    member_count: g.member_count.unwrap_or(0) as i32,
                    role: g.role.parse::<i32>().unwrap_or(0),
                    joined_at: Utc.from_utc_datetime(&g.joined_at),
                    remark: String::new(), // 群备注默认为空，将在service层填充
                });
            }

            // 获取总数
            total = sqlx::query!(
                r#"
                SELECT COUNT(*) as count
                FROM groups g
                JOIN group_members m ON g.id = m.group_id
                WHERE m.user_id = $1
                AND (g.name ILIKE $2 OR g.description ILIKE $2)
                "#,
                user_id,
                format!("%{}%", kw)
            )
            .fetch_one(&self.pool)
            .await?
            .count
            .unwrap_or(0);
        } else {
            // 无关键字时的查询
            let groups = sqlx::query!(
                r#"
                SELECT 
                    g.id,
                    g.name,
                    g.avatar_url,
                    m.role,
                    m.joined_at,
                    (SELECT COUNT(*) FROM group_members WHERE group_id = g.id) as member_count
                FROM groups g
                JOIN group_members m ON g.id = m.group_id
                WHERE m.user_id = $1
                ORDER BY g.name
                LIMIT $2 OFFSET $3
                "#,
                user_id,
                page_size as i64,
                offset as i64
            )
            .fetch_all(&self.pool)
            .await?;

            // 将查询结果转换为UserGroup对象
            for g in groups {
                result.push(UserGroup {
                    id: g.id,
                    name: g.name,
                    avatar_url: g.avatar_url.unwrap_or_default(),
                    member_count: g.member_count.unwrap_or(0) as i32,
                    role: g.role.parse::<i32>().unwrap_or(0),
                    joined_at: Utc.from_utc_datetime(&g.joined_at),
                    remark: String::new(), // 群备注默认为空，将在service层填充
                });
            }

            // 获取总数
            total = sqlx::query!(
                r#"
                SELECT COUNT(*) as count
                FROM groups g
                JOIN group_members m ON g.id = m.group_id
                WHERE m.user_id = $1
                "#,
                user_id
            )
            .fetch_one(&self.pool)
            .await?
            .count
            .unwrap_or(0);
        }

        Ok((result, total))
    }
}
