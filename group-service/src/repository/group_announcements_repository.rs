use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;

use crate::model::group_announcement::GroupAnnouncement;

pub struct GroupAnnouncementRepository {
    pool: PgPool,
}

impl GroupAnnouncementRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // 创建群公告
    pub async fn create_announcement(
        &self,
        group_id: String,
        creator_id: String,
        title: Option<String>,
        content: String,
        is_pinned: bool,
    ) -> Result<GroupAnnouncement> {
        let announcement = GroupAnnouncement::new(group_id, creator_id, title, content, is_pinned);

        // 将DateTime<Utc>转换为NaiveDateTime
        let created_at_naive = announcement.created_at.naive_utc();
        let updated_at_naive = announcement.updated_at.naive_utc();

        let result = sqlx::query!(
            r#"
            INSERT INTO group_announcements (id, group_id, creator_id, title, content, is_pinned, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, group_id, creator_id, title, content, is_pinned, created_at, updated_at
            "#,
            announcement.id,
            announcement.group_id,
            announcement.creator_id,
            announcement.title,
            announcement.content,
            announcement.is_pinned as i32, // 将bool转为i32 (0/1)
            created_at_naive,
            updated_at_naive
        )
        .fetch_one(&self.pool)
        .await?;

        // 如果这是一个置顶公告，则更新群组的announcement_id
        if is_pinned {
            sqlx::query!(
                r#"
                UPDATE groups
                SET announcement_id = $1
                WHERE id = $2
                "#,
                announcement.id,
                announcement.group_id
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(GroupAnnouncement {
            id: result.id,
            group_id: result.group_id,
            creator_id: result.creator_id,
            title: result.title,
            content: result.content,
            is_pinned: result.is_pinned != 0, // 将i32转回bool
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }

    // 获取群公告
    pub async fn get_announcement(&self, announcement_id: String) -> Result<GroupAnnouncement> {
        let result = sqlx::query!(
            r#"
            SELECT id, group_id, creator_id, title, content, is_pinned, created_at, updated_at
            FROM group_announcements
            WHERE id = $1
            "#,
            announcement_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(GroupAnnouncement {
            id: result.id,
            group_id: result.group_id,
            creator_id: result.creator_id,
            title: result.title,
            content: result.content,
            is_pinned: result.is_pinned != 0,
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }

    // 获取群组所有公告
    pub async fn get_group_announcements(&self, group_id: String) -> Result<Vec<GroupAnnouncement>> {
        let results = sqlx::query!(
            r#"
            SELECT id, group_id, creator_id, title, content, is_pinned, created_at, updated_at
            FROM group_announcements
            WHERE group_id = $1
            ORDER BY is_pinned DESC, created_at DESC
            "#,
            group_id
        )
        .fetch_all(&self.pool)
        .await?;

        let announcements = results
            .into_iter()
            .map(|result| GroupAnnouncement {
                id: result.id,
                group_id: result.group_id,
                creator_id: result.creator_id,
                title: result.title,
                content: result.content,
                is_pinned: result.is_pinned != 0,
                created_at: Utc.from_utc_datetime(&result.created_at),
                updated_at: Utc.from_utc_datetime(&result.updated_at),
            })
            .collect();

        Ok(announcements)
    }

    // 删除群公告
    pub async fn delete_announcement(
        &self,
        announcement_id: String,
        deleted_by_id: String,
    ) -> Result<bool> {
        // 先获取公告信息，检查是否是创建者或管理员
        let announcement = self.get_announcement(announcement_id.clone()).await?;
        
        // 检查群组的当前公告ID是否是要删除的公告
        let group_result = sqlx::query!(
            r#"
            SELECT announcement_id
            FROM groups
            WHERE id = $1
            "#,
            announcement.group_id
        )
        .fetch_one(&self.pool)
        .await?;

        // 如果群组的当前公告是要删除的公告，则清除群组的announcement_id
        if let Some(current_announcement_id) = group_result.announcement_id {
            if current_announcement_id == announcement_id {
                sqlx::query!(
                    r#"
                    UPDATE groups
                    SET announcement_id = NULL
                    WHERE id = $1
                    "#,
                    announcement.group_id
                )
                .execute(&self.pool)
                .await?;
            }
        }

        // 删除公告
        let rows_affected = sqlx::query!(
            r#"
            DELETE FROM group_announcements
            WHERE id = $1
            "#,
            announcement_id
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }

    // 将公告设为置顶/取消置顶
    pub async fn toggle_pin_announcement(
        &self,
        announcement_id: String,
        is_pinned: bool,
    ) -> Result<GroupAnnouncement> {
        let now = Utc::now();
        let now_naive = now.naive_utc();

        // 获取当前公告信息
        let current = self.get_announcement(announcement_id.clone()).await?;

        // 如果要设为置顶，先取消所有其他置顶公告
        if is_pinned {
            sqlx::query!(
                r#"
                UPDATE group_announcements
                SET is_pinned = 0, updated_at = $1
                WHERE group_id = $2 AND id != $3 AND is_pinned = 1
                "#,
                now_naive,
                current.group_id,
                announcement_id
            )
            .execute(&self.pool)
            .await?;

            // 更新群组的当前公告ID
            sqlx::query!(
                r#"
                UPDATE groups
                SET announcement_id = $1
                WHERE id = $2
                "#,
                announcement_id,
                current.group_id
            )
            .execute(&self.pool)
            .await?;
        } else {
            // 如果取消置顶且群组的当前公告是这个公告，则清除群组的announcement_id
            let group_result = sqlx::query!(
                r#"
                SELECT announcement_id
                FROM groups
                WHERE id = $1
                "#,
                current.group_id
            )
            .fetch_one(&self.pool)
            .await?;

            if let Some(current_announcement_id) = group_result.announcement_id {
                if current_announcement_id == announcement_id {
                    sqlx::query!(
                        r#"
                        UPDATE groups
                        SET announcement_id = NULL
                        WHERE id = $1
                        "#,
                        current.group_id
                    )
                    .execute(&self.pool)
                    .await?;
                }
            }
        }

        // 更新公告置顶状态
        let result = sqlx::query!(
            r#"
            UPDATE group_announcements
            SET is_pinned = $1, updated_at = $2
            WHERE id = $3
            RETURNING id, group_id, creator_id, title, content, is_pinned, created_at, updated_at
            "#,
            is_pinned as i32,
            now_naive,
            announcement_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(GroupAnnouncement {
            id: result.id,
            group_id: result.group_id,
            creator_id: result.creator_id,
            title: result.title,
            content: result.content,
            is_pinned: result.is_pinned != 0,
            created_at: Utc.from_utc_datetime(&result.created_at),
            updated_at: Utc.from_utc_datetime(&result.updated_at),
        })
    }
} 