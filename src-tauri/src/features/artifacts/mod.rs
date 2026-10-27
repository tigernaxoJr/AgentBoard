use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Artifact {
    pub id: String,
    pub task_id: String,
    pub agent_id: String,
    pub file_path: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateArtifactDto {
    pub agent_id: String,
    pub file_path: String,
    pub summary: String,
}

pub struct ArtifactService;

impl ArtifactService {
    pub async fn create_artifact(
        pool: &Pool<Sqlite>,
        task_id: &str,
        dto: CreateArtifactDto,
    ) -> Result<Artifact, sqlx::Error> {
        let artifact = Artifact {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            agent_id: dto.agent_id,
            file_path: dto.file_path,
            summary: dto.summary,
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query(
            r#"
            INSERT INTO artifacts (id, task_id, agent_id, file_path, summary, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&artifact.id)
        .bind(&artifact.task_id)
        .bind(&artifact.agent_id)
        .bind(&artifact.file_path)
        .bind(&artifact.summary)
        .bind(&artifact.created_at)
        .execute(pool)
        .await?;

        Ok(artifact)
    }

    pub async fn list_by_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
    ) -> Result<Vec<Artifact>, sqlx::Error> {
        let items = sqlx::query_as::<_, Artifact>(
            "SELECT * FROM artifacts WHERE task_id = ? ORDER BY created_at DESC",
        )
        .bind(task_id)
        .fetch_all(pool)
        .await?;
        Ok(items)
    }
}
