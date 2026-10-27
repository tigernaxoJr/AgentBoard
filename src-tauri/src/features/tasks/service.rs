use super::domain::Task;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Deserialize)]
pub struct CreateTaskDto {
    pub title: String,
    pub prompt: String,
    pub creator: Option<String>,
    pub assigned_agent: Option<String>,
    pub depends_on_task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimTaskDto {
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AddTaskLogDto {
    pub agent_id: String,
    pub level: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskLog {
    pub id: String,
    pub task_id: String,
    pub agent_id: String,
    pub level: String,
    pub message: String,
    pub created_at: String,
}

pub struct TaskService;

impl TaskService {
    /// 建立任務
    pub async fn create_task(pool: &Pool<Sqlite>, dto: CreateTaskDto) -> Result<Task, sqlx::Error> {
        let task = Task::new(
            dto.title,
            dto.prompt,
            dto.creator.unwrap_or_else(|| "human".to_string()),
            dto.assigned_agent,
            dto.depends_on_task_id,
        );

        sqlx::query(
            r#"
            INSERT INTO tasks (id, title, prompt, status, assigned_agent, creator, depends_on_task_id, last_heartbeat_at, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&task.id)
        .bind(&task.title)
        .bind(&task.prompt)
        .bind(&task.status)
        .bind(&task.assigned_agent)
        .bind(&task.creator)
        .bind(&task.depends_on_task_id)
        .bind(&task.last_heartbeat_at)
        .bind(&task.created_at)
        .bind(&task.updated_at)
        .execute(pool)
        .await?;

        Ok(task)
    }

    /// 列出所有任務
    pub async fn list_tasks(pool: &Pool<Sqlite>) -> Result<Vec<Task>, sqlx::Error> {
        let tasks = sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks ORDER BY created_at DESC"
        )
        .fetch_all(pool)
        .await?;
        Ok(tasks)
    }

    /// 取得單一任務
    pub async fn get_task(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Task>, sqlx::Error> {
        let task = sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        Ok(task)
    }

    /// 原子性領取任務（Claim）
    /// 條件：
    /// 1. status = 'queued'
    /// 2. assigned_agent IS NULL 或等於此 agent_id
    /// 3. 若有 depends_on_task_id，該依賴任務必須為 'completed'
    pub async fn claim_task(pool: &Pool<Sqlite>, agent_id: &str) -> Result<Option<Task>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();

        // 使用 SQLite Transaction 搭配原子更新取得唯一領取權
        let mut tx = pool.begin().await?;

        let candidate_id: Option<String> = sqlx::query_scalar(
            r#"
            SELECT t.id FROM tasks t
            WHERE t.status = 'queued'
              AND (t.assigned_agent IS NULL OR t.assigned_agent = ?)
              AND (
                t.depends_on_task_id IS NULL 
                OR EXISTS (
                  SELECT 1 FROM tasks dep 
                  WHERE dep.id = t.depends_on_task_id AND dep.status = 'completed'
                )
              )
            ORDER BY t.created_at ASC
            LIMIT 1
            "#,
        )
        .bind(agent_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(task_id) = candidate_id {
            sqlx::query(
                r#"
                UPDATE tasks 
                SET status = 'running', assigned_agent = ?, last_heartbeat_at = ?, updated_at = ?
                WHERE id = ? AND status = 'queued'
                "#,
            )
            .bind(agent_id)
            .bind(&now)
            .bind(&now)
            .bind(&task_id)
            .execute(&mut *tx)
            .await?;

            let claimed_task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
                .bind(&task_id)
                .fetch_one(&mut *tx)
                .await?;

            tx.commit().await?;
            Ok(Some(claimed_task))
        } else {
            tx.commit().await?;
            Ok(None)
        }
    }

    /// 任務心跳更新
    pub async fn heartbeat(pool: &Pool<Sqlite>, task_id: &str, agent_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            r#"
            UPDATE tasks
            SET last_heartbeat_at = ?, updated_at = ?
            WHERE id = ? AND status = 'running' AND assigned_agent = ?
            "#,
        )
        .bind(&now)
        .bind(&now)
        .bind(task_id)
        .bind(agent_id)
        .execute(pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }

    /// 完成任務
    pub async fn complete_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'completed', updated_at = ? WHERE id = ? AND status = 'running'"
        )
        .bind(&now)
        .bind(task_id)
        .execute(pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }

    /// 失敗任務
    pub async fn fail_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'failed', updated_at = ? WHERE id = ? AND status = 'running'"
        )
        .bind(&now)
        .bind(task_id)
        .execute(pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }

    /// 逾時檢查與重置（Background Worker 呼叫）
    pub async fn recover_timed_out_tasks(pool: &Pool<Sqlite>, timeout_seconds: i64) -> Result<u64, sqlx::Error> {
        let threshold = Utc::now() - chrono::Duration::seconds(timeout_seconds);
        let threshold_str = threshold.to_rfc3339();
        let now = Utc::now().to_rfc3339();

        let rows = sqlx::query(
            r#"
            UPDATE tasks
            SET status = 'queued', last_heartbeat_at = NULL, updated_at = ?
            WHERE status = 'running' 
              AND (
                (last_heartbeat_at IS NOT NULL AND last_heartbeat_at < ?)
                OR (last_heartbeat_at IS NULL AND updated_at < ?)
              )
            "#,
        )
        .bind(&now)
        .bind(&threshold_str)
        .bind(&threshold_str)
        .execute(pool)
        .await?
        .rows_affected();

        Ok(rows)
    }

    /// 新增日誌
    pub async fn add_log(
        pool: &Pool<Sqlite>,
        task_id: &str,
        dto: AddTaskLogDto,
    ) -> Result<TaskLog, sqlx::Error> {
        let log = TaskLog {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            agent_id: dto.agent_id,
            level: dto.level.unwrap_or_else(|| "INFO".to_string()),
            message: dto.message,
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query(
            r#"
            INSERT INTO task_logs (id, task_id, agent_id, level, message, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&log.id)
        .bind(&log.task_id)
        .bind(&log.agent_id)
        .bind(&log.level)
        .bind(&log.message)
        .bind(&log.created_at)
        .execute(pool)
        .await?;

        Ok(log)
    }

    /// 取得任務的所有日誌
    pub async fn get_logs(pool: &Pool<Sqlite>, task_id: &str) -> Result<Vec<TaskLog>, sqlx::Error> {
        let logs = sqlx::query_as::<_, TaskLog>(
            "SELECT * FROM task_logs WHERE task_id = ? ORDER BY created_at ASC"
        )
        .bind(task_id)
        .fetch_all(pool)
        .await?;
        Ok(logs)
    }
}
