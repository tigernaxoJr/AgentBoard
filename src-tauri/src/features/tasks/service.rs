use super::domain::Task;
use crate::features::agents;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Default, Deserialize)]
pub struct CreateTaskDto {
    pub title: String,
    pub prompt: String,
    pub creator: Option<String>,
    pub assigned_agent: Option<String>,
    pub depends_on_task_id: Option<String>,
    pub model: Option<String>,
    pub workdir: Option<String>,
    pub auto_approve: Option<bool>,
    pub isolate: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimTaskDto {
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct LeaseDto {
    pub lease_id: String,
}

/// 執行設定（派發 / 直接執行時一併寫入任務）
/// 欄位省略 = 保留原值；model / workdir 傳空字串 = 清除
#[derive(Debug, Default, Deserialize)]
pub struct RunOptions {
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub workdir: Option<String>,
    pub auto_approve: Option<bool>,
    pub isolate: Option<bool>,
}

/// 領取成功時回傳的任務，附帶僅領取者知道的租約 ID，以及要執行的 argv
#[derive(Debug, Serialize)]
pub struct ClaimedTask {
    #[serde(flatten)]
    pub task: Task,
    pub lease_id: String,
    /// 由 Agent Registry 組出；不支援的 Agent 為 None
    pub command: Option<Vec<String>>,
    /// 實際執行目錄：隔離執行時為 worktree 內對應的目錄，否則等於 workdir
    pub run_dir: Option<String>,
}

impl ClaimedTask {
    fn new(task: Task, lease_id: String) -> Self {
        let command = task.assigned_agent.as_deref().and_then(|agent| agents::command_for(agent, &task));
        let run_dir = task.workdir.clone();
        Self { task, lease_id, command, run_dir }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequeueResult {
    Requeued { downstream: u64 },
    /// 任務不存在或不是 failed
    NotFailed,
    /// 前置任務也是 failed，重新排隊也永遠領不到
    DependencyFailed,
}

#[derive(Debug, Default, Serialize)]
pub struct RecoveryResult {
    pub requeued: u64,
    pub failed: u64,
}

/// 任務最多被領取幾次，逾時重試超過即視為失敗
pub const MAX_ATTEMPTS: i64 = 3;

/// 看板列表最多顯示幾筆已結束的任務
pub const FINISHED_LIST_LIMIT: i64 = 200;

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
        let mut task = Task::new(
            dto.title,
            dto.prompt,
            dto.creator.unwrap_or_else(|| "human".to_string()),
            dto.assigned_agent,
            dto.depends_on_task_id,
        );
        task.model = dto.model.filter(|m| !m.is_empty());
        task.workdir = dto.workdir.filter(|w| !w.is_empty());
        task.auto_approve = dto.auto_approve.unwrap_or(false);
        task.isolate = dto.isolate.unwrap_or(false);

        sqlx::query(
            r#"
            INSERT INTO tasks (id, title, prompt, status, assigned_agent, creator, depends_on_task_id, last_heartbeat_at, model, workdir, auto_approve, isolate, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(&task.model)
        .bind(&task.workdir)
        .bind(task.auto_approve)
        .bind(task.isolate)
        .bind(&task.created_at)
        .bind(&task.updated_at)
        .execute(pool)
        .await?;

        Ok(task)
    }

    /// 列出任務：排隊中 / 執行中全部回傳，已結束的只回傳最近 FINISHED_LIST_LIMIT 筆
    pub async fn list_tasks(pool: &Pool<Sqlite>) -> Result<Vec<Task>, sqlx::Error> {
        let tasks = sqlx::query_as::<_, Task>(
            r#"
            SELECT * FROM tasks
            WHERE status IN ('queued', 'running')
               OR id IN (
                 SELECT id FROM tasks WHERE status IN ('completed', 'failed')
                 ORDER BY updated_at DESC LIMIT ?
               )
            ORDER BY created_at DESC
            "#,
        )
        .bind(FINISHED_LIST_LIMIT)
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

        Ok(task.map(|task| ClaimedTask::new(task, lease_id)))
    }

    /// 領取指定任務（方案 A：面板直接執行）
    /// 同時寫入執行設定；任務必須在佇列中且前置任務已完成，否則回傳 None
    pub async fn claim_specific(
        pool: &Pool<Sqlite>,
        task_id: &str,
        agent_id: &str,
        opts: &RunOptions,
    ) -> Result<Option<ClaimedTask>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let lease_id = uuid::Uuid::new_v4().to_string();

        let task = sqlx::query_as::<_, Task>(
            r#"
            UPDATE tasks
            SET status = 'running', assigned_agent = ?1, lease_id = ?2,
                attempts = attempts + 1, last_heartbeat_at = ?3, updated_at = ?3,
                model = NULLIF(COALESCE(?4, model), ''),
                workdir = NULLIF(COALESCE(?5, workdir), ''),
                auto_approve = COALESCE(?6, auto_approve),
                isolate = COALESCE(?8, isolate)
            WHERE id = ?7 AND status = 'queued'
              AND (
                depends_on_task_id IS NULL
                OR EXISTS (SELECT 1 FROM tasks dep WHERE dep.id = tasks.depends_on_task_id AND dep.status = 'completed')
              )
            RETURNING *
            "#,
        )
        .bind(agent_id)
        .bind(&lease_id)
        .bind(&now)
        .bind(&opts.model)
        .bind(&opts.workdir)
        .bind(opts.auto_approve)
        .bind(task_id)
        .bind(opts.isolate)
        .fetch_optional(pool)
        .await?;

        Ok(task.map(|task| ClaimedTask::new(task, lease_id)))
    }

    /// 手動派發：寫入執行設定並把排隊中的任務提到最前面，讓 Agent 下次輪詢就領到它
    pub async fn dispatch_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
        opts: &RunOptions,
    ) -> Result<Option<Task>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        sqlx::query_as::<_, Task>(
            r#"
            UPDATE tasks
            SET priority = (SELECT COALESCE(MAX(priority), 0) + 1 FROM tasks),
                assigned_agent = COALESCE(?, assigned_agent),
                model = NULLIF(COALESCE(?, model), ''),
                workdir = NULLIF(COALESCE(?, workdir), ''),
                auto_approve = COALESCE(?, auto_approve),
                isolate = COALESCE(?, isolate),
                updated_at = ?
            WHERE id = ? AND status = 'queued'
            RETURNING *
            "#,
        )
        .bind(&opts.agent_id)
        .bind(&opts.model)
        .bind(&opts.workdir)
        .bind(opts.auto_approve)
        .bind(opts.isolate)
        .bind(&now)
        .bind(task_id)
        .fetch_optional(pool)
        .await
    }

    /// 記錄（或清除）隔離執行用的 worktree 資訊：(路徑, 分支, 起點 commit)
    pub async fn set_worktree(
        pool: &Pool<Sqlite>,
        task_id: &str,
        worktree: Option<(&str, &str, &str)>,
    ) -> Result<(), sqlx::Error> {
        let (path, branch, base) = worktree.map_or((None, None, None), |(p, b, c)| (Some(p), Some(b), Some(c)));
        sqlx::query("UPDATE tasks SET worktree_path = ?, branch = ?, base_commit = ? WHERE id = ?")
            .bind(path)
            .bind(branch)
            .bind(base)
            .bind(task_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// 取消：排隊中或執行中的任務直接標記失敗並作廢租約
    /// 持有舊租約的執行者（Spawner / Runner）下次心跳會被拒，進而中止子程序
    pub async fn cancel_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'failed', lease_id = NULL, updated_at = ? WHERE id = ? AND status IN ('queued', 'running')"
        )
        .bind(&now)
        .bind(task_id)
        .execute(pool)
        .await?
        .rows_affected();

        if rows > 0 {
            Self::cascade_dependency_failures(pool).await?;
        }
        Ok(rows > 0)
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

    /// 重新排隊失敗的任務：attempts 歸零，並一併恢復因它連鎖失敗的下游任務
    /// （attempts = 0 代表從未被領取，只是被連帶標記失敗）
    pub async fn requeue_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<RequeueResult, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let mut tx = pool.begin().await?;

        let dependency_failed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tasks t JOIN tasks dep ON dep.id = t.depends_on_task_id WHERE t.id = ? AND dep.status = 'failed')",
        )
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await?;
        if dependency_failed {
            return Ok(RequeueResult::DependencyFailed);
        }

        let rows = sqlx::query(
            r#"
            UPDATE tasks SET status = 'queued', attempts = 0, lease_id = NULL, last_heartbeat_at = NULL, updated_at = ?
            WHERE id = ? AND status = 'failed'
            "#,
        )
        .bind(&now)
        .bind(task_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if rows == 0 {
            return Ok(RequeueResult::NotFailed);
        }

        let downstream = sqlx::query(
            r#"
            WITH RECURSIVE downstream(id) AS (
                SELECT id FROM tasks WHERE depends_on_task_id = ?1 AND status = 'failed' AND attempts = 0
                UNION
                SELECT t.id FROM tasks t JOIN downstream d ON t.depends_on_task_id = d.id
                WHERE t.status = 'failed' AND t.attempts = 0
            )
            UPDATE tasks SET status = 'queued', updated_at = ?2 WHERE id IN (SELECT id FROM downstream)
            "#,
        )
        .bind(task_id)
        .bind(&now)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        tx.commit().await?;
        Ok(RequeueResult::Requeued { downstream })
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
                depends_on_task_id: dep.map(Into::into),
                ..Default::default()
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

        let dispatched = TaskService::dispatch_task(&pool, &newer.id, &agy_opts()).await.unwrap().unwrap();
        assert_eq!(dispatched.assigned_agent.as_deref(), Some("agy"));
        assert!(dispatched.priority > 0);

        // 指定給 agy 後，其他 Agent 領不到它，agy 則優先領到它
        let other = TaskService::claim_task(&pool, "claude").await.unwrap().unwrap();
        assert_ne!(other.task.id, newer.id);
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!(claimed.task.id, newer.id);

        // 已在執行中的任務不能再派發
        assert!(TaskService::dispatch_task(&pool, &newer.id, &RunOptions::default()).await.unwrap().is_none());
    }

    fn agy_opts() -> RunOptions {
        RunOptions { agent_id: Some("agy".into()), ..Default::default() }
    }

    #[tokio::test]
    async fn dispatch_options_keep_omitted_fields_and_clear_empty_ones() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let opts = RunOptions { model: Some("opus".into()), workdir: Some("/tmp".into()), auto_approve: Some(true), ..agy_opts() };
        TaskService::dispatch_task(&pool, &task.id, &opts).await.unwrap().unwrap();

        // 只改 model 為空字串：清除 model，其餘保留
        let cleared = TaskService::dispatch_task(&pool, &task.id, &RunOptions { model: Some("".into()), ..Default::default() })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cleared.model, None);
        assert_eq!(cleared.workdir.as_deref(), Some("/tmp"));
        assert!(cleared.auto_approve);
        assert_eq!(cleared.assigned_agent.as_deref(), Some("agy"));
    }

    #[tokio::test]
    async fn claim_returns_command_from_registry() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let opts = RunOptions { model: Some("haiku".into()), agent_id: Some("claude".into()), ..Default::default() };
        TaskService::dispatch_task(&pool, &task.id, &opts).await.unwrap();

        let claimed = TaskService::claim_task(&pool, "claude").await.unwrap().unwrap();
        assert_eq!(
            claimed.command.unwrap(),
            ["claude", "--model", "haiku", "--permission-mode", "acceptEdits", "-p", "p"]
        );
        // 不在 Registry 裡的 Agent 也能領取，但沒有 command
        new_task(&pool, "b", None).await;
        assert!(TaskService::claim_task(&pool, "custom").await.unwrap().unwrap().command.is_none());
    }

    #[tokio::test]
    async fn claim_specific_respects_queue_state_and_dependencies() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;

        // 前置任務未完成：不能直接執行
        assert!(TaskService::claim_specific(&pool, &b.id, "agy", &agy_opts()).await.unwrap().is_none());

        let opts = RunOptions { model: Some("gemini-4-argon-high".into()), ..Default::default() };
        let claimed = TaskService::claim_specific(&pool, &a.id, "agy", &opts).await.unwrap().unwrap();
        assert_eq!(claimed.task.model.as_deref(), Some("gemini-4-argon-high"));
        assert_eq!(claimed.task.assigned_agent.as_deref(), Some("agy"));
        // 已在執行中：Runner 與第二次 spawn 都拿不到
        assert!(TaskService::claim_specific(&pool, &a.id, "agy", &opts).await.unwrap().is_none());
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_none());

        assert!(TaskService::complete_task(&pool, &a.id, &claimed.lease_id).await.unwrap());
        assert!(TaskService::claim_specific(&pool, &b.id, "claude", &RunOptions::default()).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn cancel_revokes_lease_and_fails_dependents() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();

        assert!(TaskService::cancel_task(&pool, &a.id).await.unwrap());
        assert!(!TaskService::heartbeat(&pool, &a.id, &claimed.lease_id).await.unwrap());
        assert!(!TaskService::complete_task(&pool, &a.id, &claimed.lease_id).await.unwrap());
        assert_eq!(status(&pool, &a.id).await, "failed");
        assert_eq!(status(&pool, &b.id).await, "failed");
        // 已結束的任務不能再取消
        assert!(!TaskService::cancel_task(&pool, &a.id).await.unwrap());
    }

    #[tokio::test]
    async fn requeue_restores_task_and_cascaded_dependents() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let c = new_task(&pool, "c", Some(&b.id)).await;
        for _ in 0..MAX_ATTEMPTS {
            TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
            expire_all(&pool).await;
        }
        assert_eq!(status(&pool, &c.id).await, "failed");

        // 下游任務的前置任務仍是 failed，不能單獨重排
        assert_eq!(TaskService::requeue_task(&pool, &b.id).await.unwrap(), RequeueResult::DependencyFailed);
        assert_eq!(TaskService::requeue_task(&pool, &a.id).await.unwrap(), RequeueResult::Requeued { downstream: 2 });
        for id in [&a.id, &b.id, &c.id] {
            assert_eq!(status(&pool, id).await, "queued");
        }
        // attempts 歸零，可以再被領取 MAX_ATTEMPTS 次
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!((claimed.task.id.as_str(), claimed.task.attempts), (a.id.as_str(), 1));
        assert_eq!(TaskService::requeue_task(&pool, &a.id).await.unwrap(), RequeueResult::NotFailed);
    }

    #[tokio::test]
    async fn requeue_task_whose_dependency_completed() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        // a 完成、b 自己執行失敗：重排 b 只恢復 b 本身
        let ca = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        TaskService::complete_task(&pool, &a.id, &ca.lease_id).await.unwrap();
        let cb = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        TaskService::fail_task(&pool, &b.id, &cb.lease_id).await.unwrap();
        assert_eq!(TaskService::requeue_task(&pool, &b.id).await.unwrap(), RequeueResult::Requeued { downstream: 0 });
        assert_eq!(status(&pool, &b.id).await, "queued");
    }

    #[tokio::test]
    async fn list_keeps_all_active_tasks_but_caps_finished_ones() {
        let pool = memory_pool().await;
        let total = FINISHED_LIST_LIMIT + 5;
        for i in 0..total {
            let t = new_task(&pool, &format!("done{i}"), None).await;
            TaskService::cancel_task(&pool, &t.id).await.unwrap();
        }
        let queued = new_task(&pool, "queued", None).await;

        let listed = TaskService::list_tasks(&pool).await.unwrap();
        assert_eq!(listed.len() as i64, FINISHED_LIST_LIMIT + 1);
        assert!(listed.iter().any(|t| t.id == queued.id));
        // 最早結束的幾筆被排除
        assert!(!listed.iter().any(|t| t.title == "done0"));
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
