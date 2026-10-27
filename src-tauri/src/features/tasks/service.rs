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
pub struct LeaseDto {
    pub lease_id: String,
}

#[derive(Debug, Deserialize)]
pub struct DispatchTaskDto {
    pub agent_id: Option<String>,
}

/// 領取成功時回傳的任務，附帶僅領取者知道的租約 ID
#[derive(Debug, Serialize)]
pub struct ClaimedTask {
    #[serde(flatten)]
    pub task: Task,
    pub lease_id: String,
}

#[derive(Debug, Default, Serialize)]
pub struct RecoveryResult {
    pub requeued: u64,
    pub failed: u64,
}

/// 任務最多被領取幾次，逾時重試超過即視為失敗
pub const MAX_ATTEMPTS: i64 = 3;

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
    /// 以單一 UPDATE ... RETURNING 完成挑選與標記，並發領取時不會重複發出同一任務
    pub async fn claim_task(pool: &Pool<Sqlite>, agent_id: &str) -> Result<Option<ClaimedTask>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let lease_id = uuid::Uuid::new_v4().to_string();

        let task = sqlx::query_as::<_, Task>(
            r#"
            UPDATE tasks
            SET status = 'running', assigned_agent = ?1, lease_id = ?2,
                attempts = attempts + 1, last_heartbeat_at = ?3, updated_at = ?3
            WHERE status = 'queued' AND id = (
                SELECT t.id FROM tasks t
                WHERE t.status = 'queued'
                  AND (t.assigned_agent IS NULL OR t.assigned_agent = ?1)
                  AND (
                    t.depends_on_task_id IS NULL
                    OR EXISTS (
                      SELECT 1 FROM tasks dep
                      WHERE dep.id = t.depends_on_task_id AND dep.status = 'completed'
                    )
                  )
                ORDER BY t.priority DESC, t.created_at ASC
                LIMIT 1
            )
            RETURNING *
            "#,
        )
        .bind(agent_id)
        .bind(&lease_id)
        .bind(&now)
        .fetch_optional(pool)
        .await?;

        Ok(task.map(|task| ClaimedTask { task, lease_id }))
    }

    /// 手動派發：把排隊中的任務提到最前面（可同時改指定 Agent），讓 Agent 下次輪詢就領到它
    pub async fn dispatch_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Option<Task>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        sqlx::query_as::<_, Task>(
            r#"
            UPDATE tasks
            SET priority = (SELECT COALESCE(MAX(priority), 0) + 1 FROM tasks),
                assigned_agent = COALESCE(?, assigned_agent),
                updated_at = ?
            WHERE id = ? AND status = 'queued'
            RETURNING *
            "#,
        )
        .bind(agent_id)
        .bind(&now)
        .bind(task_id)
        .fetch_optional(pool)
        .await
    }

    /// 任務心跳更新（必須持有目前的租約）
    pub async fn heartbeat(pool: &Pool<Sqlite>, task_id: &str, lease_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            r#"
            UPDATE tasks
            SET last_heartbeat_at = ?, updated_at = ?
            WHERE id = ? AND status = 'running' AND lease_id = ?
            "#,
        )
        .bind(&now)
        .bind(&now)
        .bind(task_id)
        .bind(lease_id)
        .execute(pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }

    /// 完成任務（必須持有目前的租約）
    pub async fn complete_task(pool: &Pool<Sqlite>, task_id: &str, lease_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'completed', lease_id = NULL, updated_at = ? WHERE id = ? AND status = 'running' AND lease_id = ?"
        )
        .bind(&now)
        .bind(task_id)
        .bind(lease_id)
        .execute(pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }

    /// 失敗任務（必須持有目前的租約）；依賴它的排隊任務會一併標記失敗
    pub async fn fail_task(pool: &Pool<Sqlite>, task_id: &str, lease_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'failed', lease_id = NULL, updated_at = ? WHERE id = ? AND status = 'running' AND lease_id = ?"
        )
        .bind(&now)
        .bind(task_id)
        .bind(lease_id)
        .execute(pool)
        .await?
        .rows_affected();

        if rows > 0 {
            Self::cascade_dependency_failures(pool).await?;
        }
        Ok(rows > 0)
    }

    /// 逾時檢查與重置（Background Worker 呼叫）
    /// 未達 MAX_ATTEMPTS 的放回佇列重試，已達上限的標記 failed
    pub async fn recover_timed_out_tasks(pool: &Pool<Sqlite>, timeout_seconds: i64) -> Result<RecoveryResult, sqlx::Error> {
        let threshold_str = (Utc::now() - chrono::Duration::seconds(timeout_seconds)).to_rfc3339();
        let now = Utc::now().to_rfc3339();

        let expired: Vec<(String, i64)> = sqlx::query_as(
            r#"
            UPDATE tasks
            SET status = CASE WHEN attempts >= ? THEN 'failed' ELSE 'queued' END,
                lease_id = NULL, last_heartbeat_at = NULL, updated_at = ?
            WHERE status = 'running'
              AND (
                (last_heartbeat_at IS NOT NULL AND last_heartbeat_at < ?)
                OR (last_heartbeat_at IS NULL AND updated_at < ?)
              )
            RETURNING status, attempts
            "#,
        )
        .bind(MAX_ATTEMPTS)
        .bind(&now)
        .bind(&threshold_str)
        .bind(&threshold_str)
        .fetch_all(pool)
        .await?;

        let mut result = RecoveryResult::default();
        for (status, _) in &expired {
            if status == "failed" { result.failed += 1 } else { result.requeued += 1 }
        }
        if result.failed > 0 {
            result.failed += Self::cascade_dependency_failures(pool).await?;
        }
        Ok(result)
    }

    /// 前置任務失敗時，依賴鏈上仍在排隊的任務永遠無法執行，直接標記失敗
    async fn cascade_dependency_failures(pool: &Pool<Sqlite>) -> Result<u64, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let mut total = 0;
        loop {
            let rows = sqlx::query(
                r#"
                UPDATE tasks SET status = 'failed', updated_at = ?
                WHERE status = 'queued'
                  AND depends_on_task_id IN (SELECT id FROM tasks WHERE status = 'failed')
                "#,
            )
            .bind(&now)
            .execute(pool)
            .await?
            .rows_affected();
            if rows == 0 {
                return Ok(total);
            }
            total += rows;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    // :memory: 每條連線各自一個資料庫，所以只開一條
    async fn memory_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    async fn new_task(pool: &Pool<Sqlite>, title: &str, dep: Option<&str>) -> Task {
        TaskService::create_task(
            pool,
            CreateTaskDto {
                title: title.into(),
                prompt: "p".into(),
                creator: None,
                assigned_agent: None,
                depends_on_task_id: dep.map(Into::into),
            },
        )
        .await
        .unwrap()
    }

    async fn status(pool: &Pool<Sqlite>, id: &str) -> String {
        TaskService::get_task(pool, id).await.unwrap().unwrap().status
    }

    /// 讓所有 running 任務立即視為逾時
    async fn expire_all(pool: &Pool<Sqlite>) -> RecoveryResult {
        TaskService::recover_timed_out_tasks(pool, -1).await.unwrap()
    }

    #[tokio::test]
    async fn only_lease_holder_can_heartbeat_and_complete() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!(claimed.task.id, task.id);
        assert_eq!(claimed.task.attempts, 1);

        assert!(!TaskService::heartbeat(&pool, &task.id, "wrong").await.unwrap());
        assert!(!TaskService::complete_task(&pool, &task.id, "wrong").await.unwrap());
        assert!(!TaskService::fail_task(&pool, &task.id, "wrong").await.unwrap());
        assert!(TaskService::heartbeat(&pool, &task.id, &claimed.lease_id).await.unwrap());
        assert!(TaskService::complete_task(&pool, &task.id, &claimed.lease_id).await.unwrap());
        assert_eq!(status(&pool, &task.id).await, "completed");
    }

    #[tokio::test]
    async fn stale_runner_cannot_complete_reclaimed_task() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let first = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!(expire_all(&pool).await.requeued, 1);

        let second = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_ne!(first.lease_id, second.lease_id);
        assert!(!TaskService::complete_task(&pool, &task.id, &first.lease_id).await.unwrap());
        assert!(!TaskService::heartbeat(&pool, &task.id, &first.lease_id).await.unwrap());
        assert_eq!(status(&pool, &task.id).await, "running");
        assert!(TaskService::complete_task(&pool, &task.id, &second.lease_id).await.unwrap());
    }

    #[tokio::test]
    async fn timed_out_task_fails_after_max_attempts() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        for attempt in 1..=MAX_ATTEMPTS {
            let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
            assert_eq!(claimed.task.attempts, attempt);
            let result = expire_all(&pool).await;
            if attempt < MAX_ATTEMPTS {
                assert_eq!((result.requeued, result.failed), (1, 0));
                assert_eq!(status(&pool, &task.id).await, "queued");
            } else {
                assert_eq!((result.requeued, result.failed), (0, 1));
            }
        }
        assert_eq!(status(&pool, &task.id).await, "failed");
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_dependency_fails_whole_chain() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let c = new_task(&pool, "c", Some(&b.id)).await;
        let other = new_task(&pool, "unrelated", None).await;

        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!(claimed.task.id, a.id);
        assert!(TaskService::fail_task(&pool, &a.id, &claimed.lease_id).await.unwrap());

        assert_eq!(status(&pool, &b.id).await, "failed");
        assert_eq!(status(&pool, &c.id).await, "failed");
        assert_eq!(status(&pool, &other.id).await, "queued");
    }

    #[tokio::test]
    async fn dependency_failing_by_timeout_also_cascades() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        for _ in 0..MAX_ATTEMPTS {
            TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
            expire_all(&pool).await;
        }
        assert_eq!(status(&pool, &a.id).await, "failed");
        assert_eq!(status(&pool, &b.id).await, "failed");
    }

    #[tokio::test]
    async fn dispatch_moves_task_to_front_of_queue() {
        let pool = memory_pool().await;
        let _older = new_task(&pool, "older", None).await;
        let newer = new_task(&pool, "newer", None).await;

        let dispatched = TaskService::dispatch_task(&pool, &newer.id, Some("agy")).await.unwrap().unwrap();
        assert_eq!(dispatched.assigned_agent.as_deref(), Some("agy"));
        assert!(dispatched.priority > 0);

        // 指定給 agy 後，其他 Agent 領不到它，agy 則優先領到它
        let other = TaskService::claim_task(&pool, "claude").await.unwrap().unwrap();
        assert_ne!(other.task.id, newer.id);
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!(claimed.task.id, newer.id);

        // 已在執行中的任務不能再派發
        assert!(TaskService::dispatch_task(&pool, &newer.id, None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn concurrent_claims_never_hand_out_same_task() {
        let path = std::env::temp_dir().join(format!("agent_board_test_{}.db", uuid::Uuid::new_v4()));
        let pool = crate::db::init_db(path.to_str().unwrap()).await.unwrap();
        for i in 0..3 {
            new_task(&pool, &format!("t{i}"), None).await;
        }

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let pool = pool.clone();
                tokio::spawn(async move { TaskService::claim_task(&pool, &format!("agent{i}")).await.unwrap() })
            })
            .collect();
        let mut ids = Vec::new();
        for h in handles {
            if let Some(c) = h.await.unwrap() {
                ids.push(c.task.id);
            }
        }
        pool.close().await;
        let _ = std::fs::remove_file(&path);

        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }
}
