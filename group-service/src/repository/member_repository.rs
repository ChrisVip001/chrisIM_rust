use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;
use common::proto::group::MemberRole;
use chrono::{Utc, TimeZone};

use crate::model::member::Member;

pub struct MemberRepository {
    pool: PgPool,
}

impl MemberRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    // 添加群组成员
    pub async fn add_member(&self, group_id: Uuid, user_id: Uuid, username: String, nickname: Option<String>, 
                        avatar_url: Option<String>, role: MemberRole) -> Result<Member> {
        let member = Member::new(group_id, user_id, username, nickname, avatar_url, role);
        
        // 将DateTime<Utc>转换为NaiveDateTime
        let joined_at_naive = member.joined_at.naive_utc();
        
        let result = sqlx::query!(
            r#"
            INSERT INTO group_members (id, group_id, user_id, role, joined_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, group_id, user_id, role, joined_at
            "#,
            member.id.to_string(),
            member.group_id.to_string(),
            member.user_id.to_string(),
            member.role.to_string(),
            joined_at_naive
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Member {
            id: Uuid::parse_str(&result.id).unwrap(),
            group_id: Uuid::parse_str(&result.group_id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            username: member.username,
            nickname: member.nickname,
            avatar_url: member.avatar_url,
            role: result.role.parse::<i32>().unwrap_or(0),
            joined_at: Utc.from_utc_datetime(&result.joined_at),
        })
    }
    
    // 移除群组成员
    pub async fn remove_member(&self, group_id: Uuid, user_id: Uuid, removed_by_id: Uuid) -> Result<bool> {
        // 验证移除权限
        let remover_role = self.get_member_role(group_id, removed_by_id).await?;
        let member_role = self.get_member_role(group_id, user_id).await?;
        
        if remover_role < MemberRole::Admin as i32 {
            return Err(anyhow::anyhow!("没有权限移除成员"));
        }
        
        if remover_role <= member_role && removed_by_id != user_id {
            return Err(anyhow::anyhow!("无法移除同级或更高级别的成员"));
        }
        
        let rows_affected = sqlx::query!(
            r#"
            DELETE FROM group_members
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id.to_string(),
            user_id.to_string()
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        
        Ok(rows_affected > 0)
    }
    
    // 更新成员角色
    pub async fn update_member_role(&self, group_id: Uuid, user_id: Uuid, updated_by_id: Uuid, role: MemberRole) -> Result<Member> {
        // 验证更新权限
        let updater_role = self.get_member_role(group_id, updated_by_id).await?;
        let _member_role = self.get_member_role(group_id, user_id).await?;
        
        if updater_role < MemberRole::Owner as i32 {
            return Err(anyhow::anyhow!("只有群主可以更新成员角色"));
        }
        
        if role as i32 >= updater_role {
            return Err(anyhow::anyhow!("无法将成员提升为与自己相同或更高的角色"));
        }
        
        // 获取用户信息
        let member_info = self.get_member(group_id, user_id).await?;
        
        // 更新角色
        let result = sqlx::query!(
            r#"
            UPDATE group_members
            SET role = $1
            WHERE group_id = $2 AND user_id = $3
            RETURNING id, group_id, user_id, role, joined_at
            "#,
            (role as i32).to_string(),
            group_id.to_string(),
            user_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Member {
            id: Uuid::parse_str(&result.id).unwrap(),
            group_id: Uuid::parse_str(&result.group_id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            username: member_info.username,
            nickname: member_info.nickname,
            avatar_url: member_info.avatar_url,
            role: result.role.parse::<i32>().unwrap_or(0),
            joined_at: Utc.from_utc_datetime(&result.joined_at),
        })
    }
    
    // 获取群组成员
    pub async fn get_member(&self, group_id: Uuid, user_id: Uuid) -> Result<Member> {
        // 在真实环境中，这需要从user-service获取用户信息
        // 这里简化处理，仅从数据库获取基本信息
        let result = sqlx::query!(
            r#"
            SELECT m.id, m.group_id, m.user_id, m.role, m.joined_at, 
                   u.username, u.nickname, u.avatar_url
            FROM group_members m
            JOIN users u ON m.user_id = u.id
            WHERE m.group_id = $1 AND m.user_id = $2
            "#,
            group_id.to_string(),
            user_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Member {
            id: Uuid::parse_str(&result.id).unwrap(),
            group_id: Uuid::parse_str(&result.group_id).unwrap(),
            user_id: Uuid::parse_str(&result.user_id).unwrap(),
            username: result.username,
            nickname: result.nickname,
            avatar_url: result.avatar_url,
            role: result.role.parse::<i32>().unwrap_or(0),
            joined_at: Utc.from_utc_datetime(&result.joined_at),
        })
    }
    
    // 获取成员角色
    pub async fn get_member_role(&self, group_id: Uuid, user_id: Uuid) -> Result<i32> {
        let result = sqlx::query!(
            r#"
            SELECT role
            FROM group_members
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id.to_string(),
            user_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await?;
        
        match result {
            Some(r) => Ok(r.role.parse::<i32>().unwrap_or(0)),
            None => Err(anyhow::anyhow!("用户不是群组成员")),
        }
    }
    
    // 获取群组成员列表
    pub async fn get_members(&self, group_id: Uuid) -> Result<Vec<Member>> {
        // 在真实环境中，这需要从user-service获取用户信息
        let members = sqlx::query!(
            r#"
            SELECT m.id, m.group_id, m.user_id, m.role, m.joined_at,
                   u.username, u.nickname, u.avatar_url
            FROM group_members m
            JOIN users u ON m.user_id = u.id
            WHERE m.group_id = $1
            ORDER BY m.role DESC, m.joined_at ASC
            "#,
            group_id.to_string()
        )
        .fetch_all(&self.pool)
        .await?;
        
        let result = members
            .into_iter()
            .map(|m| Member {
                id: Uuid::parse_str(&m.id).unwrap(),
                group_id: Uuid::parse_str(&m.group_id).unwrap(),
                user_id: Uuid::parse_str(&m.user_id).unwrap(),
                username: m.username,
                nickname: m.nickname,
                avatar_url: m.avatar_url,
                role: m.role.parse::<i32>().unwrap_or(0),
                joined_at: Utc.from_utc_datetime(&m.joined_at),
            })
            .collect();
        
        Ok(result)
    }
    
    // 检查用户是否是群组成员
    pub async fn check_membership(&self, group_id: Uuid, user_id: Uuid) -> Result<(bool, Option<i32>)> {
        let result = sqlx::query!(
            r#"
            SELECT role
            FROM group_members
            WHERE group_id = $1 AND user_id = $2
            "#,
            group_id.to_string(),
            user_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await?;
        
        match result {
            Some(r) => Ok((true, Some(r.role.parse::<i32>().unwrap_or(0)))),
            None => Ok((false, None)),
        }
    }
}