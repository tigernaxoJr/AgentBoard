use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskTemplate {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub title: String,
    pub prompt: String,
    pub assigned_agent: Option<String>,
    pub model: Option<String>,
    pub workdir: Option<String>,
    pub auto_approve: bool,
    pub isolate: bool,
    pub priority: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateDto {
    pub name: String,
    pub description: Option<String>,
    pub title: String,
    pub prompt: String,
    pub assigned_agent: Option<String>,
    pub model: Option<String>,
    pub workdir: Option<String>,
    pub auto_approve: Option<bool>,
    pub isolate: Option<bool>,
    pub priority: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateDto {
    pub name: Option<String>,
    pub description: Option<String>,
    pub title: Option<String>,
    pub prompt: Option<String>,
    pub assigned_agent: Option<String>,
    pub model: Option<String>,
    pub workdir: Option<String>,
    pub auto_approve: Option<bool>,
    pub isolate: Option<bool>,
    pub priority: Option<i64>,
}

pub struct TemplateService;

impl TemplateService {
    pub async fn list(pool: &Pool<Sqlite>) -> Result<Vec<TaskTemplate>, sqlx::Error> {
        sqlx::query_as::<_, TaskTemplate>(
            r#"
            SELECT id, name, description, title, prompt, assigned_agent, model, workdir,
                   auto_approve, isolate, priority, created_at, updated_at
            FROM task_templates
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(pool)
        .await
    }

    pub async fn get(pool: &Pool<Sqlite>, id: &str) -> Result<Option<TaskTemplate>, sqlx::Error> {
        sqlx::query_as::<_, TaskTemplate>(
            r#"
            SELECT id, name, description, title, prompt, assigned_agent, model, workdir,
                   auto_approve, isolate, priority, created_at, updated_at
            FROM task_templates
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &Pool<Sqlite>,
        dto: CreateTemplateDto,
    ) -> Result<TaskTemplate, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        let auto_approve = dto.auto_approve.unwrap_or(true);
        let isolate = dto.isolate.unwrap_or(false);
        let priority = dto.priority.unwrap_or(0);

        sqlx::query(
            r#"
            INSERT INTO task_templates (
                id, name, description, title, prompt, assigned_agent, model, workdir,
                auto_approve, isolate, priority, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&dto.name)
        .bind(&dto.description)
        .bind(&dto.title)
        .bind(&dto.prompt)
        .bind(&dto.assigned_agent)
        .bind(&dto.model)
        .bind(&dto.workdir)
        .bind(auto_approve)
        .bind(isolate)
        .bind(priority)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Ok(TaskTemplate {
            id,
            name: dto.name,
            description: dto.description,
            title: dto.title,
            prompt: dto.prompt,
            assigned_agent: dto.assigned_agent,
            model: dto.model,
            workdir: dto.workdir,
            auto_approve,
            isolate,
            priority,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub async fn update(
        pool: &Pool<Sqlite>,
        id: &str,
        dto: UpdateTemplateDto,
    ) -> Result<Option<TaskTemplate>, sqlx::Error> {
        let existing = Self::get(pool, id).await?;
        let Some(t) = existing else { return Ok(None) };

        let now = Utc::now().to_rfc3339();
        let name = dto.name.unwrap_or(t.name);
        // 選填欄位：未提供 = 保留原值，空字串 = 清空
        let merge = |new: Option<String>, old: Option<String>| match new {
            Some(v) if v.trim().is_empty() => None,
            Some(v) => Some(v),
            None => old,
        };
        let description = merge(dto.description, t.description);
        let title = dto.title.unwrap_or(t.title);
        let prompt = dto.prompt.unwrap_or(t.prompt);
        let assigned_agent = merge(dto.assigned_agent, t.assigned_agent);
        let model = merge(dto.model, t.model);
        let workdir = merge(dto.workdir, t.workdir);
        let auto_approve = dto.auto_approve.unwrap_or(t.auto_approve);
        let isolate = dto.isolate.unwrap_or(t.isolate);
        let priority = dto.priority.unwrap_or(t.priority);

        sqlx::query(
            r#"
            UPDATE task_templates
            SET name = ?, description = ?, title = ?, prompt = ?, assigned_agent = ?,
                model = ?, workdir = ?, auto_approve = ?, isolate = ?, priority = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&name)
        .bind(&description)
        .bind(&title)
        .bind(&prompt)
        .bind(&assigned_agent)
        .bind(&model)
        .bind(&workdir)
        .bind(auto_approve)
        .bind(isolate)
        .bind(priority)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;

        Self::get(pool, id).await
    }

    pub async fn delete(pool: &Pool<Sqlite>, id: &str) -> Result<bool, sqlx::Error> {
        let rows = sqlx::query("DELETE FROM task_templates WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn update_can_clear_optional_fields() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrate(&pool).await.unwrap();
        let t = TemplateService::create(
            &pool,
            CreateTemplateDto {
                name: "n".into(),
                description: Some("d".into()),
                title: "t".into(),
                prompt: "p".into(),
                assigned_agent: None,
                model: Some("m".into()),
                workdir: None,
                auto_approve: None,
                isolate: None,
                priority: None,
            },
        )
        .await
        .unwrap();
        let u = TemplateService::update(
            &pool,
            &t.id,
            UpdateTemplateDto {
                name: None,
                description: Some("".into()),
                title: None,
                prompt: None,
                assigned_agent: None,
                model: None,
                workdir: None,
                auto_approve: None,
                isolate: None,
                priority: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(u.description, None);
        assert_eq!(u.model.as_deref(), Some("m"));
    }
}
