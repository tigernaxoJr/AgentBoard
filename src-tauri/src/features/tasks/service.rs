use super::domain::Task;
use crate::features::agents;
use crate::features::agents::stream::{RunResult, Usage};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Default, Deserialize)]
pub struct CreateTaskDto {
    pub title: String,
    pub prompt: String,
    pub creator: Option<String>,
    pub assigned_agent: Option<String>,
    /// 依賴的任務 ID 陣列
    pub dependency_ids: Option<Vec<String>>,
    pub model: Option<String>,
    pub workdir: Option<String>,
    pub auto_approve: Option<bool>,
    pub isolate: Option<bool>,
    /// 接續既有的 CLI session（面板列出的外部 session）
    pub session_id: Option<String>,
    /// 領取優先序，越大越先被領取（預設 0）
    pub priority: Option<i64>,
    /// true = 建立後先進入 pending_approval，由人類在看板核准才會排入佇列
    pub requires_approval: Option<bool>,
    /// 由伺服器依建立者目前執行中的任務推算，不接受外部傳入
    #[serde(skip)]
    pub depth: i64,
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

/// Prompt 內的這個字串在執行時換成前置任務的結果，用來串接多個任務
pub const DEP_RESULT_PLACEHOLDER: &str = "{{dep.result}}";

impl ClaimedTask {
    async fn new(pool: &Pool<Sqlite>, task: Task, lease_id: String) -> Result<Self, sqlx::Error> {
        // 只替換實際執行的 runnable argv，Task 本身的 prompt 保留範本原樣
        let rendered_prompt = Self::render_prompt(pool, &task).await?;
        let mut runnable = task.clone();
        runnable.prompt = rendered_prompt;
        let command = task
            .assigned_agent
            .as_deref()
            .and_then(|agent| agents::command_for(agent, &runnable));
        let run_dir = task.workdir.clone();
        Ok(Self {
            task,
            lease_id,
            command,
            run_dir,
        })
    }

    async fn render_prompt(pool: &Pool<Sqlite>, task: &Task) -> Result<String, sqlx::Error> {
        let mut prompt = task.prompt.clone();
        if !prompt.contains("{{dep") {
            return Ok(prompt);
        }

        // 取得所有關聯的前置任務結果
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT dep.id, dep.result FROM task_dependencies td
            JOIN tasks dep ON dep.id = td.depends_on_task_id
            WHERE td.task_id = ?1
            "#,
        )
        .bind(&task.id)
        .fetch_all(pool)
        .await?;

        // 1. 支援個別短 ID 替換：{{dep.<短ID>.result}}
        for (id, res) in &rows {
            let short = if id.len() >= 8 { &id[..8] } else { id.as_str() };
            let key = format!("{{{{dep.{}.result}}}}", short);
            let val = res
                .as_deref()
                .filter(|r| !r.trim().is_empty())
                .unwrap_or("（無結果）");
            prompt = prompt.replace(&key, val);
        }

        // 2. 替換通用 {{dep.result}}：若有多個前置任務則合併格式化，若單一前置則直接替換
        if prompt.contains(DEP_RESULT_PLACEHOLDER) {
            let combined = if rows.is_empty() {
                "（無前置任務結果）".to_string()
            } else if rows.len() == 1 {
                rows[0]
                    .1
                    .as_deref()
                    .filter(|r| !r.trim().is_empty())
                    .unwrap_or("（前置任務沒有回報結果）")
                    .to_string()
            } else {
                rows.iter()
                    .map(|(id, res)| {
                        let short = if id.len() >= 8 { &id[..8] } else { id.as_str() };
                        let val = res
                            .as_deref()
                            .filter(|r| !r.trim().is_empty())
                            .unwrap_or("（無結果）");
                        format!("### 前置任務 #{short} 結果：\n{val}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            prompt = prompt.replace(DEP_RESULT_PLACEHOLDER, &combined);
        }

        Ok(prompt)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequeueResult {
    Requeued {
        downstream: u64,
    },
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
/// 逾時後重新排隊的退避：第 n 次嘗試後等 n * 這麼多秒才能被再次領取
pub const RETRY_BACKOFF_SECS: i64 = 30;
/// Agent 派發 Agent 的最大層數，超過即拒絕建立
pub const MAX_DEPTH: i64 = 5;
/// 同一個 Agent 連續失敗這麼多次後，Runner 暫停領取，直到人類恢復
pub const PAUSE_AFTER_FAILURES: i64 = 3;
/// 失敗原因的字數上限
const MAX_ERROR_CHARS: usize = 500;

/// 看板列表最多顯示幾筆已結束的任務
pub const FINISHED_LIST_LIMIT: i64 = 200;
/// 每個任務最多保留的日誌筆數，超過時刪除最舊的
pub const MAX_LOGS_PER_TASK: i64 = 2000;
/// 單則日誌的字數上限
const MAX_LOG_CHARS: usize = 8000;
/// 每新增這麼多則日誌才修剪一次，避免每次寫入都掃描
const LOG_TRIM_EVERY: u64 = 100;
static LOG_INSERTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        let mut dep_ids: Vec<String> = Vec::new();
        for d in dto.dependency_ids.unwrap_or_default() {
            let d = d.trim().to_string();
            if !d.is_empty() && !dep_ids.contains(&d) {
                dep_ids.push(d);
            }
        }
        let mut task = Task::new(
            dto.title,
            dto.prompt,
            dto.creator.unwrap_or_else(|| "human".to_string()),
            dto.assigned_agent,
        );
        task.dependency_ids = dep_ids.clone();
        task.model = dto.model.filter(|m| !m.is_empty());
        task.workdir = dto.workdir.filter(|w| !w.is_empty());
        task.auto_approve = dto.auto_approve.unwrap_or(false);
        task.isolate = dto.isolate.unwrap_or(false);
        task.session_id = dto.session_id.filter(|s| !s.is_empty());
        task.priority = dto.priority.unwrap_or(0);
        task.depth = dto.depth;
        if dto.requires_approval.unwrap_or(false) {
            task.status = "pending_approval".to_string();
        }

        // 任務與依賴同一個交易：依賴 ID 不存在（FK 失敗）時整筆回滾，避免產生「無依賴」的任務被直接執行
        let mut tx = pool.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO tasks (id, title, prompt, status, assigned_agent, creator, last_heartbeat_at, model, workdir, auto_approve, isolate, session_id, priority, depth, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&task.id)
        .bind(&task.title)
        .bind(&task.prompt)
        .bind(&task.status)
        .bind(&task.assigned_agent)
        .bind(&task.creator)
        .bind(&task.last_heartbeat_at)
        .bind(&task.model)
        .bind(&task.workdir)
        .bind(task.auto_approve)
        .bind(task.isolate)
        .bind(&task.session_id)
        .bind(task.priority)
        .bind(task.depth)
        .bind(&task.created_at)
        .bind(&task.updated_at)
        .execute(&mut *tx)
        .await?;

        for dep_id in &dep_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_task_id) VALUES (?, ?)"
            )
            .bind(&task.id)
            .bind(dep_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        Ok(task)
    }

    /// 新任務的派發深度：建立者是某個 Agent 時，等於它目前執行中任務的最大深度 + 1；
    /// 其他情況（人類、沒有執行中任務）為 0
    pub async fn derived_depth(pool: &Pool<Sqlite>, creator: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COALESCE(MAX(depth) + 1, 0) FROM tasks WHERE status = 'running' AND assigned_agent = ?",
        )
        .bind(creator)
        .fetch_one(pool)
        .await
    }

    /// 核准待核准的任務，排入佇列
    pub async fn approve_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<bool, sqlx::Error> {
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'queued', updated_at = ? WHERE id = ? AND status = 'pending_approval'",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(task_id)
        .execute(pool)
        .await?
        .rows_affected();
        Ok(rows > 0)
    }

    /// 最近 PAUSE_AFTER_FAILURES 筆實際執行過的任務（恢復時間之後）全部失敗 = 暫停領取
    /// 取消與連鎖失敗（從未被領取）不算
    pub async fn is_agent_paused(pool: &Pool<Sqlite>, agent_id: &str) -> Result<bool, sqlx::Error> {
        let failed: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM (
                SELECT status FROM tasks
                WHERE assigned_agent = ?1 AND status IN ('completed', 'failed') AND attempts > 0
                  AND COALESCE(error, '') != '已取消'
                  AND updated_at > COALESCE((SELECT resumed_at FROM agent_resume WHERE agent_id = ?1), '')
                ORDER BY updated_at DESC LIMIT ?2
            ) WHERE status = 'failed'
            "#,
        )
        .bind(agent_id)
        .bind(PAUSE_AFTER_FAILURES)
        .fetch_one(pool)
        .await?;
        Ok(failed >= PAUSE_AFTER_FAILURES)
    }

    pub async fn paused_agents(pool: &Pool<Sqlite>) -> Result<Vec<String>, sqlx::Error> {
        let agents: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT assigned_agent FROM tasks WHERE assigned_agent IS NOT NULL",
        )
        .fetch_all(pool)
        .await?;
        let mut paused = Vec::new();
        for agent in agents {
            if Self::is_agent_paused(pool, &agent).await? {
                paused.push(agent);
            }
        }
        Ok(paused)
    }

    /// 人類恢復被暫停的 Agent：之後的失敗重新計數
    pub async fn resume_agent(pool: &Pool<Sqlite>, agent_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO agent_resume (agent_id, resumed_at) VALUES (?1, ?2) \
             ON CONFLICT(agent_id) DO UPDATE SET resumed_at = ?2",
        )
        .bind(agent_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn populate_dependencies(
        pool: &Pool<Sqlite>,
        tasks: &mut [Task],
    ) -> Result<(), sqlx::Error> {
        if tasks.is_empty() {
            return Ok(());
        }
        // 只查這批任務的依賴，而不是整張表
        let ids = serde_json::to_string(&tasks.iter().map(|t| &t.id).collect::<Vec<_>>())
            .unwrap_or_default();
        let deps: Vec<(String, String)> = sqlx::query_as(
            "SELECT task_id, depends_on_task_id FROM task_dependencies WHERE task_id IN (SELECT value FROM json_each(?))",
        )
        .bind(ids)
        .fetch_all(pool)
        .await?;

        let mut by_task: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (tid, did) in deps {
            by_task.entry(tid).or_default().push(did);
        }
        for t in tasks.iter_mut() {
            t.dependency_ids = by_task.remove(&t.id).unwrap_or_default();
        }
        Ok(())
    }

    /// 列出任務：排隊中 / 執行中全部回傳，已結束的只回傳最近 FINISHED_LIST_LIMIT 筆
    /// 有搜尋字串時改為搜尋全部任務（標題、Prompt、ID、結果），回傳最近 FINISHED_LIST_LIMIT 筆
    pub async fn list_tasks(
        pool: &Pool<Sqlite>,
        query: Option<&str>,
    ) -> Result<Vec<Task>, sqlx::Error> {
        let query = query.map(str::trim).filter(|q| !q.is_empty());
        let sql = if query.is_some() {
            r#"
            SELECT * FROM tasks
            WHERE instr(lower(title || char(10) || prompt || char(10) || id || char(10) || coalesce(result, '')), lower(?2)) > 0
            ORDER BY created_at DESC LIMIT ?1
            "#
        } else {
            r#"
            SELECT * FROM tasks
            WHERE status IN ('queued', 'running', 'pending_approval')
               OR id IN (
                 SELECT id FROM tasks WHERE status IN ('completed', 'failed')
                 ORDER BY updated_at DESC LIMIT ?1
               )
            ORDER BY created_at DESC
            "#
        };
        let q = sqlx::query_as::<_, Task>(sql).bind(FINISHED_LIST_LIMIT);
        let mut tasks = match query {
            Some(text) => q.bind(text).fetch_all(pool).await?,
            None => q.fetch_all(pool).await?,
        };
        Self::populate_dependencies(pool, &mut tasks).await?;
        Ok(tasks)
    }

    /// 取得單一任務
    pub async fn get_task(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Task>, sqlx::Error> {
        let task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        if let Some(t) = task {
            let mut list = vec![t];
            Self::populate_dependencies(pool, &mut list).await?;
            Ok(list.into_iter().next())
        } else {
            Ok(None)
        }
    }

    /// 原子性領取任務（Claim）
    /// 條件：
    /// 1. status = 'queued'
    /// 2. assigned_agent IS NULL 或等於此 agent_id
    /// 3. task_dependencies 內的所有前置任務都必須為 'completed'
    ///
    /// 以單一 UPDATE ... RETURNING 完成挑選與標記，並發領取時不會重複發出同一任務
    pub async fn claim_task(
        pool: &Pool<Sqlite>,
        agent_id: &str,
    ) -> Result<Option<ClaimedTask>, sqlx::Error> {
        if Self::is_agent_paused(pool, agent_id).await? {
            return Ok(None);
        }
        let now = Utc::now().to_rfc3339();
        let lease_id = uuid::Uuid::new_v4().to_string();

        let task = sqlx::query_as::<_, Task>(
            r#"
            UPDATE tasks
            SET status = 'running', assigned_agent = ?1, lease_id = ?2,
                result = NULL, cost_usd = NULL, input_tokens = NULL, output_tokens = NULL, duration_ms = NULL,
                error = NULL, not_before = NULL,
                attempts = attempts + 1, last_heartbeat_at = ?3, updated_at = ?3
            WHERE status = 'queued' AND id = (
                SELECT t.id FROM tasks t
                WHERE t.status = 'queued'
                  AND (t.assigned_agent IS NULL OR t.assigned_agent = ?1)
                  AND (t.not_before IS NULL OR t.not_before <= ?3)
                  AND NOT EXISTS (
                    SELECT 1 FROM task_dependencies td
                    JOIN tasks dep ON dep.id = td.depends_on_task_id
                    WHERE td.task_id = t.id AND dep.status != 'completed'
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

        match task {
            Some(task) => Ok(Some(ClaimedTask::new(pool, task, lease_id).await?)),
            None => Ok(None),
        }
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
                result = NULL, cost_usd = NULL, input_tokens = NULL, output_tokens = NULL, duration_ms = NULL,
                error = NULL, not_before = NULL,
                attempts = attempts + 1, last_heartbeat_at = ?3, updated_at = ?3,
                model = NULLIF(COALESCE(?4, model), ''),
                workdir = NULLIF(COALESCE(?5, workdir), ''),
                auto_approve = COALESCE(?6, auto_approve),
                isolate = COALESCE(?8, isolate)
            WHERE id = ?7 AND status = 'queued'
              AND NOT EXISTS (
                SELECT 1 FROM task_dependencies td
                JOIN tasks dep ON dep.id = td.depends_on_task_id
                WHERE td.task_id = tasks.id AND dep.status != 'completed'
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

        match task {
            Some(task) => Ok(Some(ClaimedTask::new(pool, task, lease_id).await?)),
            None => Ok(None),
        }
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
        let (path, branch, base) =
            worktree.map_or((None, None, None), |(p, b, c)| (Some(p), Some(b), Some(c)));
        sqlx::query("UPDATE tasks SET worktree_path = ?, branch = ?, base_commit = ? WHERE id = ?")
            .bind(path)
            .bind(branch)
            .bind(base)
            .bind(task_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// 記錄 Agent 的工作階段 ID，追加指示或重試時用來接續
    pub async fn set_session(
        pool: &Pool<Sqlite>,
        task_id: &str,
        session_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE tasks SET session_id = ? WHERE id = ?")
            .bind(session_id)
            .bind(task_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// 記錄這次執行的結果；結果事件沒提供的數字保留逐步累加的值（每次領取時清空）
    pub async fn set_result(
        pool: &Pool<Sqlite>,
        task_id: &str,
        r: &RunResult,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE tasks SET result = ?, cost_usd = COALESCE(?, cost_usd), input_tokens = COALESCE(?, input_tokens),
                             output_tokens = COALESCE(?, output_tokens), duration_ms = COALESCE(?, duration_ms)
            WHERE id = ?
            "#,
        )
        .bind(&r.text)
        .bind(r.cost_usd)
        .bind(r.input_tokens)
        .bind(r.output_tokens)
        .bind(r.duration_ms)
        .bind(task_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// 累加這次執行的用量（agy 的結果事件是整段對話的累計，只能逐步加總）
    pub async fn add_usage(
        pool: &Pool<Sqlite>,
        task_id: &str,
        u: &Usage,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE tasks SET input_tokens = COALESCE(input_tokens, 0) + ?, output_tokens = COALESCE(output_tokens, 0) + ?,
                             duration_ms = COALESCE(duration_ms, 0) + ?
            WHERE id = ?
            "#,
        )
        .bind(u.input_tokens)
        .bind(u.output_tokens)
        .bind(u.duration_ms)
        .bind(task_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// 是否仍持有執行中任務的租約
    pub async fn holds_lease(
        pool: &Pool<Sqlite>,
        task_id: &str,
        lease_id: &str,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM tasks WHERE id = ? AND status = 'running' AND lease_id = ?",
        )
        .bind(task_id)
        .bind(lease_id)
        .fetch_one(pool)
        .await
    }

    /// 取消：排隊中或執行中的任務直接標記失敗並作廢租約
    /// 持有舊租約的執行者（Spawner / Runner）下次心跳會被拒，進而中止子程序
    pub async fn cancel_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'failed', lease_id = NULL, error = '已取消', updated_at = ? WHERE id = ? AND status IN ('queued', 'running', 'pending_approval')"
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

    /// 徹底刪除任務及其相關依賴、日誌與產出記錄
    /// 尚未完成的任務被刪除時，下游排隊中的任務永遠等不到它，先標記失敗；
    /// 否則依賴列被刪掉後，下游會變成「無依賴」而被直接領取執行
    pub async fn delete_task(pool: &Pool<Sqlite>, task_id: &str) -> Result<bool, sqlx::Error> {
        let mut tx = pool.begin().await?;

        sqlx::query(
            r#"
            WITH RECURSIVE downstream(id) AS (
                SELECT task_id FROM task_dependencies WHERE depends_on_task_id = ?1
                UNION
                SELECT td.task_id FROM task_dependencies td JOIN downstream d ON td.depends_on_task_id = d.id
            )
            UPDATE tasks SET status = 'failed', error = '前置任務已被刪除', updated_at = ?2
            WHERE status IN ('queued', 'pending_approval') AND id IN (SELECT id FROM downstream)
              AND EXISTS (SELECT 1 FROM tasks WHERE id = ?1 AND status != 'completed')
            "#,
        )
        .bind(task_id)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;

        // 追加指示的子任務不隨原任務刪除，只是不再指向它
        sqlx::query("UPDATE tasks SET parent_task_id = NULL WHERE parent_task_id = ?")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;

        // 刪除關聯資料
        sqlx::query("DELETE FROM task_logs WHERE task_id = ?").bind(task_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM artifacts WHERE task_id = ?").bind(task_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM task_dependencies WHERE task_id = ? OR depends_on_task_id = ?")
            .bind(task_id)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;

        let rows = sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(task_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();

        tx.commit().await?;
        Ok(rows > 0)
    }

    /// 任務心跳更新（必須持有目前的租約）
    pub async fn heartbeat(
        pool: &Pool<Sqlite>,
        task_id: &str,
        lease_id: &str,
    ) -> Result<bool, sqlx::Error> {
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
    pub async fn complete_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
        lease_id: &str,
    ) -> Result<bool, sqlx::Error> {
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

        if rows > 0 {
            // 把輸出資料夾內的檔案登記為成果；失敗不影響任務完成
            let row: Option<(Option<String>, Option<String>, Option<String>)> =
                sqlx::query_as("SELECT worktree_path, workdir, assigned_agent FROM tasks WHERE id = ?")
                    .bind(task_id)
                    .fetch_optional(pool)
                    .await?;
            if let Some((worktree, workdir, agent)) = row {
                let roots: Vec<&str> = [worktree.as_deref(), workdir.as_deref()].into_iter().flatten().collect();
                let _ = crate::features::artifacts::register_outputs(
                    pool,
                    task_id,
                    agent.as_deref().unwrap_or("agent"),
                    &roots,
                )
                .await;
            }
        }

        Ok(rows > 0)
    }

    /// 失敗任務（必須持有目前的租約）；依賴它的排隊任務會一併標記失敗
    pub async fn fail_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
        lease_id: &str,
    ) -> Result<bool, sqlx::Error> {
        Self::fail_task_with_reason(pool, task_id, lease_id, None).await
    }

    /// Agent 在結果事件裡自己回報失敗（程序結束碼可能仍是 0）：不需租約，只要任務還在執行中
    /// 之後 Runner / Spawner 再回報 complete 會因為任務已不在執行中而被忽略
    pub async fn fail_running_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
        reason: &str,
    ) -> Result<bool, sqlx::Error> {
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'failed', lease_id = NULL, error = ?, updated_at = ? WHERE id = ? AND status = 'running'",
        )
        .bind(agents::stream::truncate(reason, MAX_ERROR_CHARS))
        .bind(Utc::now().to_rfc3339())
        .bind(task_id)
        .execute(pool)
        .await?
        .rows_affected();
        if rows > 0 {
            Self::cascade_dependency_failures(pool).await?;
        }
        Ok(rows > 0)
    }

    /// 同 `fail_task`，並記錄失敗原因
    pub async fn fail_task_with_reason(
        pool: &Pool<Sqlite>,
        task_id: &str,
        lease_id: &str,
        reason: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let reason = reason.map(|r| agents::stream::truncate(r, MAX_ERROR_CHARS));
        let rows = sqlx::query(
            "UPDATE tasks SET status = 'failed', lease_id = NULL, error = ?, updated_at = ? WHERE id = ? AND status = 'running' AND lease_id = ?"
        )
        .bind(&reason)
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

    /// 追加指示：以已結束的任務為基礎建立新任務，沿用同一個 Agent、執行設定與工作階段
    /// 原任務的 worktree 移交給新任務，Agent 才能在同一份修改上接著做
    /// 原任務不在已結束狀態或沒有指定 Agent 時回傳 None
    pub async fn create_followup(
        pool: &Pool<Sqlite>,
        parent_id: &str,
        prompt: &str,
    ) -> Result<Option<Task>, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let mut tx = pool.begin().await?;
        let task: Option<Task> = sqlx::query_as(
            r#"
            INSERT INTO tasks (id, title, prompt, status, assigned_agent, creator, parent_task_id, model, workdir, auto_approve, isolate,
                               worktree_path, branch, base_commit, session_id, depth, created_at, updated_at)
            SELECT ?1, CASE WHEN title LIKE '↳ %' THEN title ELSE '↳ ' || title END, ?2, 'queued', assigned_agent, 'human', id,
                   model, workdir, auto_approve, isolate, worktree_path, branch, base_commit, session_id, depth, ?3, ?3
            FROM tasks
            WHERE id = ?4 AND status IN ('completed', 'failed') AND assigned_agent IS NOT NULL
            RETURNING *
            "#,
        )
        .bind(&id)
        .bind(prompt)
        .bind(&now)
        .bind(parent_id)
        .fetch_optional(&mut *tx)
        .await?;
        if task.is_some() {
            sqlx::query("UPDATE tasks SET worktree_path = NULL, branch = NULL, base_commit = NULL WHERE id = ?")
                .bind(parent_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(task)
    }

    /// 讓排隊中的任務在指定秒數後才可被領取
    pub async fn defer_task(pool: &Pool<Sqlite>, task_id: &str, secs: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks SET not_before = strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '+' || ?1 || ' seconds') \
             WHERE id = ?2 AND status = 'queued'",
        )
        .bind(secs)
        .bind(task_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// 重新排隊失敗的任務：attempts 歸零，並一併恢復因它連鎖失敗的下游任務
    /// （attempts = 0 代表從未被領取，只是被連帶標記失敗）
    /// fresh = true 時清除工作階段，Agent 從頭執行而不是接續上一段對話
    pub async fn requeue_task(
        pool: &Pool<Sqlite>,
        task_id: &str,
        fresh: bool,
    ) -> Result<RequeueResult, sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let mut tx = pool.begin().await?;

        let dependency_failed: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM task_dependencies td
                JOIN tasks dep ON dep.id = td.depends_on_task_id
                WHERE td.task_id = ?1 AND dep.status = 'failed'
            )
            "#,
        )
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await?;
        if dependency_failed {
            return Ok(RequeueResult::DependencyFailed);
        }

        let rows = sqlx::query(
            r#"
            UPDATE tasks SET status = 'queued', attempts = 0, lease_id = NULL, last_heartbeat_at = NULL, updated_at = ?1,
                error = NULL, not_before = NULL,
                session_id = CASE WHEN ?3 THEN NULL ELSE session_id END
            WHERE id = ?2 AND status = 'failed'
            "#,
        )
        .bind(&now)
        .bind(task_id)
        .bind(fresh)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if rows == 0 {
            return Ok(RequeueResult::NotFailed);
        }

        let downstream = sqlx::query(
            r#"
            WITH RECURSIVE downstream(id) AS (
                SELECT task_id FROM task_dependencies WHERE depends_on_task_id = ?1
                UNION
                SELECT td.task_id FROM task_dependencies td JOIN downstream d ON td.depends_on_task_id = d.id
            )
            UPDATE tasks SET status = 'queued', error = NULL, updated_at = ?2
            WHERE id IN (SELECT id FROM downstream) AND status = 'failed' AND attempts = 0
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
    pub async fn recover_timed_out_tasks(
        pool: &Pool<Sqlite>,
        timeout_seconds: i64,
    ) -> Result<RecoveryResult, sqlx::Error> {
        let threshold_str = (Utc::now() - chrono::Duration::seconds(timeout_seconds)).to_rfc3339();
        let now = Utc::now().to_rfc3339();

        let expired: Vec<(String, i64)> = sqlx::query_as(
            r#"
            UPDATE tasks
            SET status = CASE WHEN attempts >= ?1 THEN 'failed' ELSE 'queued' END,
                error = CASE WHEN attempts >= ?1 THEN '租約逾時：已嘗試 ' || attempts || ' 次，Runner 都沒有回報心跳' ELSE error END,
                not_before = CASE WHEN attempts >= ?1 THEN NULL
                    ELSE strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '+' || (?4 * attempts) || ' seconds') END,
                lease_id = NULL, last_heartbeat_at = NULL, updated_at = ?2
            WHERE status = 'running'
              AND (
                (last_heartbeat_at IS NOT NULL AND last_heartbeat_at < ?3)
                OR (last_heartbeat_at IS NULL AND updated_at < ?3)
              )
            RETURNING status, attempts
            "#,
        )
        .bind(MAX_ATTEMPTS)
        .bind(&now)
        .bind(&threshold_str)
        .bind(RETRY_BACKOFF_SECS)
        .fetch_all(pool)
        .await?;

        let mut result = RecoveryResult::default();
        for (status, _) in &expired {
            if status == "failed" {
                result.failed += 1
            } else {
                result.requeued += 1
            }
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
                UPDATE tasks SET status = 'failed', error = '前置任務失敗', updated_at = ?
                WHERE status IN ('queued', 'pending_approval')
                  AND id IN (
                    SELECT td.task_id FROM task_dependencies td
                    JOIN tasks dep ON dep.id = td.depends_on_task_id
                    WHERE dep.status = 'failed'
                  )
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
            message: agents::stream::truncate(&dto.message, MAX_LOG_CHARS),
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

        if LOG_INSERTS
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .is_multiple_of(LOG_TRIM_EVERY)
        {
            Self::trim_logs(pool, task_id).await?;
        }
        Ok(log)
    }

    async fn trim_logs(pool: &Pool<Sqlite>, task_id: &str) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            DELETE FROM task_logs WHERE id IN (
                SELECT id FROM task_logs WHERE task_id = ?1 ORDER BY created_at DESC LIMIT -1 OFFSET ?2
            )
            "#,
        )
        .bind(task_id)
        .bind(MAX_LOGS_PER_TASK)
        .execute(pool)
        .await?
        .rows_affected();
        Ok(rows)
    }

    /// 取得任務最近的 MAX_LOGS_PER_TASK 則日誌（由舊到新）
    pub async fn get_logs(pool: &Pool<Sqlite>, task_id: &str) -> Result<Vec<TaskLog>, sqlx::Error> {
        let logs = sqlx::query_as::<_, TaskLog>(
            "SELECT * FROM (SELECT * FROM task_logs WHERE task_id = ? ORDER BY created_at DESC LIMIT ?) ORDER BY created_at ASC",
        )
        .bind(task_id)
        .bind(MAX_LOGS_PER_TASK)
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
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    async fn new_task(pool: &Pool<Sqlite>, title: &str, dep: Option<&str>) -> Task {
        TaskService::create_task(
            pool,
            CreateTaskDto {
                title: title.into(),
                prompt: "p".into(),
                dependency_ids: dep.map(|d| vec![d.to_string()]),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn status(pool: &Pool<Sqlite>, id: &str) -> String {
        TaskService::get_task(pool, id)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    /// 讓所有 running 任務立即視為逾時（並略過重試退避，讓測試能馬上重新領取）
    async fn expire_all(pool: &Pool<Sqlite>) -> RecoveryResult {
        let result = expire_all_keeping_backoff(pool).await;
        sqlx::query("UPDATE tasks SET not_before = NULL")
            .execute(pool)
            .await
            .unwrap();
        result
    }

    async fn expire_all_keeping_backoff(pool: &Pool<Sqlite>) -> RecoveryResult {
        TaskService::recover_timed_out_tasks(pool, -1)
            .await
            .unwrap()
    }

    async fn error_of(pool: &Pool<Sqlite>, id: &str) -> Option<String> {
        TaskService::get_task(pool, id).await.unwrap().unwrap().error
    }

    #[tokio::test]
    async fn only_lease_holder_can_heartbeat_and_complete() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let claimed = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.task.id, task.id);
        assert_eq!(claimed.task.attempts, 1);

        assert!(
            !TaskService::heartbeat(&pool, &task.id, "wrong")
                .await
                .unwrap()
        );
        assert!(
            !TaskService::complete_task(&pool, &task.id, "wrong")
                .await
                .unwrap()
        );
        assert!(
            !TaskService::fail_task(&pool, &task.id, "wrong")
                .await
                .unwrap()
        );
        assert!(
            TaskService::heartbeat(&pool, &task.id, &claimed.lease_id)
                .await
                .unwrap()
        );
        assert!(
            TaskService::complete_task(&pool, &task.id, &claimed.lease_id)
                .await
                .unwrap()
        );
        assert_eq!(status(&pool, &task.id).await, "completed");
    }

    #[tokio::test]
    async fn stale_runner_cannot_complete_reclaimed_task() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let first = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expire_all(&pool).await.requeued, 1);

        let second = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(first.lease_id, second.lease_id);
        assert!(
            !TaskService::complete_task(&pool, &task.id, &first.lease_id)
                .await
                .unwrap()
        );
        assert!(
            !TaskService::heartbeat(&pool, &task.id, &first.lease_id)
                .await
                .unwrap()
        );
        assert_eq!(status(&pool, &task.id).await, "running");
        assert!(
            TaskService::complete_task(&pool, &task.id, &second.lease_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn timed_out_task_fails_after_max_attempts() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        for attempt in 1..=MAX_ATTEMPTS {
            let claimed = TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .unwrap();
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
        assert!(
            TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn failed_dependency_fails_whole_chain() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let c = new_task(&pool, "c", Some(&b.id)).await;
        let other = new_task(&pool, "unrelated", None).await;

        let claimed = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.task.id, a.id);
        assert!(
            TaskService::fail_task(&pool, &a.id, &claimed.lease_id)
                .await
                .unwrap()
        );

        assert_eq!(status(&pool, &b.id).await, "failed");
        assert_eq!(status(&pool, &c.id).await, "failed");
        assert_eq!(status(&pool, &other.id).await, "queued");
    }

    #[tokio::test]
    async fn timed_out_task_waits_before_it_can_be_claimed_again() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert_eq!(expire_all_keeping_backoff(&pool).await.requeued, 1);

        assert_eq!(status(&pool, &task.id).await, "queued");
        assert!(
            TaskService::claim_task(&pool, "agy").await.unwrap().is_none(),
            "backoff should hold the task back"
        );

        sqlx::query("UPDATE tasks SET not_before = '2000-01-01T00:00:00.000+00:00'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn failure_reasons_are_recorded_and_cleared_on_requeue() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        assert!(
            TaskService::fail_task_with_reason(&pool, &a.id, &claimed.lease_id, Some("boom"))
                .await
                .unwrap()
        );
        assert_eq!(error_of(&pool, &a.id).await.as_deref(), Some("boom"));
        assert_eq!(error_of(&pool, &b.id).await.as_deref(), Some("前置任務失敗"));

        TaskService::requeue_task(&pool, &a.id, false).await.unwrap();
        assert_eq!(error_of(&pool, &a.id).await, None);
        assert_eq!(error_of(&pool, &b.id).await, None);
    }

    #[tokio::test]
    async fn timeout_and_cancel_leave_a_reason() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        for _ in 0..MAX_ATTEMPTS {
            TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
            expire_all(&pool).await;
        }
        assert!(error_of(&pool, &a.id).await.unwrap().contains("租約逾時"));

        let b = new_task(&pool, "b", None).await;
        assert!(TaskService::cancel_task(&pool, &b.id).await.unwrap());
        assert_eq!(error_of(&pool, &b.id).await.as_deref(), Some("已取消"));
    }

    #[tokio::test]
    async fn agent_reported_error_fails_the_task_even_if_process_would_succeed() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        let (tx, _rx) = tokio::sync::broadcast::channel(8);

        let line = r#"{"type":"result","is_error":true,"result":"rate limited"}"#;
        agents::ingest_output(&pool, &tx, &task.id, "claude", line).await;

        assert_eq!(status(&pool, &task.id).await, "failed");
        assert!(error_of(&pool, &task.id).await.unwrap().contains("rate limited"));
        // 之後 Runner 照程序結束碼回報完成，不能把它翻回 completed
        assert!(
            !TaskService::complete_task(&pool, &task.id, &claimed.lease_id)
                .await
                .unwrap()
        );
        assert_eq!(status(&pool, &task.id).await, "failed");
    }

    #[tokio::test]
    async fn deleting_unfinished_dependency_fails_downstream_instead_of_unblocking_it() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let c = new_task(&pool, "c", Some(&b.id)).await;

        assert!(TaskService::delete_task(&pool, &a.id).await.unwrap());
        assert_eq!(status(&pool, &b.id).await, "failed");
        assert_eq!(status(&pool, &c.id).await, "failed");
        assert!(
            TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn deleting_a_task_detaches_its_followups() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        sqlx::query("UPDATE tasks SET status = 'completed', assigned_agent = 'agy' WHERE id = ?")
            .bind(&a.id)
            .execute(&pool)
            .await
            .unwrap();
        let child = TaskService::create_followup(&pool, &a.id, "more")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(child.parent_task_id.as_deref(), Some(a.id.as_str()));

        assert!(TaskService::delete_task(&pool, &a.id).await.unwrap());
        let child = TaskService::get_task(&pool, &child.id).await.unwrap().unwrap();
        assert_eq!(child.parent_task_id, None);
    }

    #[tokio::test]
    async fn deferred_task_cannot_be_claimed_until_not_before() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        TaskService::defer_task(&pool, &a.id, 60).await.unwrap();
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_none());

        sqlx::query("UPDATE tasks SET not_before = '2000-01-01T00:00:00.000+00:00'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn approval_task_waits_until_approved() {
        let pool = memory_pool().await;
        let t = TaskService::create_task(
            &pool,
            CreateTaskDto {
                title: "gated".into(),
                prompt: "p".into(),
                requires_approval: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(t.status, "pending_approval");
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_none());
        assert!(TaskService::list_tasks(&pool, None).await.unwrap().iter().any(|x| x.id == t.id));

        assert!(TaskService::approve_task(&pool, &t.id).await.unwrap());
        assert!(!TaskService::approve_task(&pool, &t.id).await.unwrap());
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn pending_approval_task_can_be_cancelled() {
        let pool = memory_pool().await;
        let t = TaskService::create_task(
            &pool,
            CreateTaskDto {
                title: "gated".into(),
                prompt: "p".into(),
                requires_approval: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(TaskService::cancel_task(&pool, &t.id).await.unwrap());
        assert_eq!(status(&pool, &t.id).await, "failed");
    }

    #[tokio::test]
    async fn depth_grows_with_agent_created_tasks() {
        let pool = memory_pool().await;
        assert_eq!(TaskService::derived_depth(&pool, "human").await.unwrap(), 0);
        assert_eq!(TaskService::derived_depth(&pool, "agy").await.unwrap(), 0);

        let a = new_task(&pool, "a", None).await;
        TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        // agy 正在執行深度 0 的任務，它建立的任務深度為 1
        assert_eq!(TaskService::derived_depth(&pool, "agy").await.unwrap(), 1);
        sqlx::query("UPDATE tasks SET depth = 4 WHERE id = ?")
            .bind(&a.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(TaskService::derived_depth(&pool, "agy").await.unwrap(), 5);
    }

    #[tokio::test]
    async fn agent_pauses_after_consecutive_failures_until_resumed() {
        let pool = memory_pool().await;
        for i in 0..PAUSE_AFTER_FAILURES {
            new_task(&pool, &format!("t{i}"), None).await;
            let c = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
            TaskService::fail_task_with_reason(&pool, &c.task.id, &c.lease_id, Some("boom"))
                .await
                .unwrap();
        }
        assert!(TaskService::is_agent_paused(&pool, "agy").await.unwrap());
        assert_eq!(TaskService::paused_agents(&pool).await.unwrap(), ["agy"]);
        assert!(!TaskService::is_agent_paused(&pool, "claude").await.unwrap());

        new_task(&pool, "later", None).await;
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_none());

        TaskService::resume_agent(&pool, "agy").await.unwrap();
        assert!(!TaskService::is_agent_paused(&pool, "agy").await.unwrap());
        assert!(TaskService::claim_task(&pool, "agy").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn success_or_cancel_does_not_count_toward_pause() {
        let pool = memory_pool().await;
        for i in 0..PAUSE_AFTER_FAILURES {
            new_task(&pool, &format!("t{i}"), None).await;
            let c = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
            if i == 1 {
                TaskService::complete_task(&pool, &c.task.id, &c.lease_id).await.unwrap();
            } else {
                TaskService::fail_task_with_reason(&pool, &c.task.id, &c.lease_id, Some("boom"))
                    .await
                    .unwrap();
            }
        }
        assert!(!TaskService::is_agent_paused(&pool, "agy").await.unwrap());
    }

    #[tokio::test]
    async fn list_returns_dependencies_for_each_task() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let listed = TaskService::list_tasks(&pool, None).await.unwrap();
        let deps = |id: &str| listed.iter().find(|t| t.id == id).unwrap().dependency_ids.clone();
        assert_eq!(deps(&b.id), [a.id.clone()]);
        assert!(deps(&a.id).is_empty());
    }

    #[tokio::test]
    async fn deleting_completed_dependency_keeps_downstream_runnable() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let claimed = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        TaskService::complete_task(&pool, &a.id, &claimed.lease_id)
            .await
            .unwrap();

        assert!(TaskService::delete_task(&pool, &a.id).await.unwrap());
        assert_eq!(status(&pool, &b.id).await, "queued");
    }

    #[tokio::test]
    async fn dependency_failing_by_timeout_also_cascades() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        for _ in 0..MAX_ATTEMPTS {
            TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .unwrap();
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

        let dispatched = TaskService::dispatch_task(&pool, &newer.id, &agy_opts())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dispatched.assigned_agent.as_deref(), Some("agy"));
        assert!(dispatched.priority > 0);

        // 指定給 agy 後，其他 Agent 領不到它，agy 則優先領到它
        let other = TaskService::claim_task(&pool, "claude")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(other.task.id, newer.id);
        let claimed = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.task.id, newer.id);

        // 已在執行中的任務不能再派發
        assert!(
            TaskService::dispatch_task(&pool, &newer.id, &RunOptions::default())
                .await
                .unwrap()
                .is_none()
        );
    }

    fn agy_opts() -> RunOptions {
        RunOptions {
            agent_id: Some("agy".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn dispatch_options_keep_omitted_fields_and_clear_empty_ones() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let opts = RunOptions {
            model: Some("opus".into()),
            workdir: Some("/tmp".into()),
            auto_approve: Some(true),
            ..agy_opts()
        };
        TaskService::dispatch_task(&pool, &task.id, &opts)
            .await
            .unwrap()
            .unwrap();

        // 只改 model 為空字串：清除 model，其餘保留
        let cleared = TaskService::dispatch_task(
            &pool,
            &task.id,
            &RunOptions {
                model: Some("".into()),
                ..Default::default()
            },
        )
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
        let opts = RunOptions {
            model: Some("haiku".into()),
            agent_id: Some("claude".into()),
            ..Default::default()
        };
        TaskService::dispatch_task(&pool, &task.id, &opts)
            .await
            .unwrap();

        let claimed = TaskService::claim_task(&pool, "claude")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            claimed.command.unwrap(),
            [
                "claude",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku",
                "--permission-mode",
                "acceptEdits",
                "-p",
                &format!("p\n\n{}", agents::board_hint(&claimed.task))
            ]
        );
        // 不在 Registry 裡的 Agent 也能領取，但沒有 command
        new_task(&pool, "b", None).await;
        assert!(
            TaskService::claim_task(&pool, "custom")
                .await
                .unwrap()
                .unwrap()
                .command
                .is_none()
        );
    }

    #[tokio::test]
    async fn claim_specific_respects_queue_state_and_dependencies() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;

        // 前置任務未完成：不能直接執行
        assert!(
            TaskService::claim_specific(&pool, &b.id, "agy", &agy_opts())
                .await
                .unwrap()
                .is_none()
        );

        let opts = RunOptions {
            model: Some("gemini-4-argon-high".into()),
            ..Default::default()
        };
        let claimed = TaskService::claim_specific(&pool, &a.id, "agy", &opts)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.task.model.as_deref(), Some("gemini-4-argon-high"));
        assert_eq!(claimed.task.assigned_agent.as_deref(), Some("agy"));
        // 已在執行中：Runner 與第二次 spawn 都拿不到
        assert!(
            TaskService::claim_specific(&pool, &a.id, "agy", &opts)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .is_none()
        );

        assert!(
            TaskService::complete_task(&pool, &a.id, &claimed.lease_id)
                .await
                .unwrap()
        );
        assert!(
            TaskService::claim_specific(&pool, &b.id, "claude", &RunOptions::default())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn cancel_revokes_lease_and_fails_dependents() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        let claimed = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();

        assert!(TaskService::cancel_task(&pool, &a.id).await.unwrap());
        assert!(
            !TaskService::heartbeat(&pool, &a.id, &claimed.lease_id)
                .await
                .unwrap()
        );
        assert!(
            !TaskService::complete_task(&pool, &a.id, &claimed.lease_id)
                .await
                .unwrap()
        );
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
            TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .unwrap();
            expire_all(&pool).await;
        }
        assert_eq!(status(&pool, &c.id).await, "failed");

        // 下游任務的前置任務仍是 failed，不能單獨重排
        assert_eq!(
            TaskService::requeue_task(&pool, &b.id, false)
                .await
                .unwrap(),
            RequeueResult::DependencyFailed
        );
        assert_eq!(
            TaskService::requeue_task(&pool, &a.id, false)
                .await
                .unwrap(),
            RequeueResult::Requeued { downstream: 2 }
        );
        for id in [&a.id, &b.id, &c.id] {
            assert_eq!(status(&pool, id).await, "queued");
        }
        // attempts 歸零，可以再被領取 MAX_ATTEMPTS 次
        let claimed = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (claimed.task.id.as_str(), claimed.task.attempts),
            (a.id.as_str(), 1)
        );
        assert_eq!(
            TaskService::requeue_task(&pool, &a.id, false)
                .await
                .unwrap(),
            RequeueResult::NotFailed
        );
    }

    #[tokio::test]
    async fn requeue_task_whose_dependency_completed() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let b = new_task(&pool, "b", Some(&a.id)).await;
        // a 完成、b 自己執行失敗：重排 b 只恢復 b 本身
        let ca = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        TaskService::complete_task(&pool, &a.id, &ca.lease_id)
            .await
            .unwrap();
        let cb = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        TaskService::fail_task(&pool, &b.id, &cb.lease_id)
            .await
            .unwrap();
        assert_eq!(
            TaskService::requeue_task(&pool, &b.id, false)
                .await
                .unwrap(),
            RequeueResult::Requeued { downstream: 0 }
        );
        assert_eq!(status(&pool, &b.id).await, "queued");
    }

    #[tokio::test]
    async fn substitutes_dependency_result_into_command() {
        let pool = memory_pool().await;
        let a = new_task(&pool, "a", None).await;
        let dto = CreateTaskDto {
            title: "b".into(),
            prompt: format!("根據分析：{DEP_RESULT_PLACEHOLDER}\n開始實作"),
            assigned_agent: Some("agy".into()),
            dependency_ids: Some(vec![a.id.clone()]),
            ..Default::default()
        };
        let b = TaskService::create_task(&pool, dto).await.unwrap();
        let ca = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        let r = RunResult {
            text: "改 foo.rs".into(),
            ..Default::default()
        };
        TaskService::set_result(&pool, &a.id, &r).await.unwrap();
        TaskService::complete_task(&pool, &a.id, &ca.lease_id)
            .await
            .unwrap();

        let cb = TaskService::claim_task(&pool, "agy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cb.task.id, b.id);
        assert_eq!(
            cb.command.unwrap().last().unwrap(),
            &format!("根據分析：改 foo.rs\n開始實作\n\n{}", agents::board_hint(&cb.task))
        );
        // 資料庫保留範本
        assert!(cb.task.prompt.contains(DEP_RESULT_PLACEHOLDER));
    }

    #[tokio::test]
    async fn fresh_requeue_drops_session() {
        for (fresh, expected) in [(false, Some("sess-1")), (true, None)] {
            let pool = memory_pool().await;
            let t = new_task(&pool, "a", None).await;
            let c = TaskService::claim_task(&pool, "agy")
                .await
                .unwrap()
                .unwrap();
            TaskService::set_session(&pool, &t.id, "sess-1")
                .await
                .unwrap();
            TaskService::fail_task(&pool, &t.id, &c.lease_id)
                .await
                .unwrap();
            TaskService::requeue_task(&pool, &t.id, fresh)
                .await
                .unwrap();
            let t = TaskService::get_task(&pool, &t.id).await.unwrap().unwrap();
            assert_eq!(t.session_id.as_deref(), expected);
        }
    }

    #[tokio::test]
    async fn trims_old_logs_and_truncates_long_messages() {
        let pool = memory_pool().await;
        let t = new_task(&pool, "a", None).await;
        let log = |m: String| AddTaskLogDto {
            agent_id: "agy".into(),
            level: None,
            message: m,
        };
        for i in 0..MAX_LOGS_PER_TASK + 5 {
            TaskService::add_log(&pool, &t.id, log(format!("line {i}")))
                .await
                .unwrap();
        }
        TaskService::trim_logs(&pool, &t.id).await.unwrap();
        let logs = TaskService::get_logs(&pool, &t.id).await.unwrap();
        assert_eq!(logs.len() as i64, MAX_LOGS_PER_TASK);
        assert_eq!(
            logs.last().unwrap().message,
            format!("line {}", MAX_LOGS_PER_TASK + 4)
        );

        let long = TaskService::add_log(&pool, &t.id, log("字".repeat(MAX_LOG_CHARS + 10)))
            .await
            .unwrap();
        assert_eq!(long.message.chars().count(), MAX_LOG_CHARS + 1); // 含結尾的「…」
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

        let listed = TaskService::list_tasks(&pool, None).await.unwrap();
        assert_eq!(listed.len() as i64, FINISHED_LIST_LIMIT + 1);
        assert!(listed.iter().any(|t| t.id == queued.id));
        // 最早結束的幾筆被排除
        assert!(!listed.iter().any(|t| t.title == "done0"));
        // 搜尋涵蓋全部任務，包含被排除的舊任務與結果文字
        let found = TaskService::list_tasks(&pool, Some(" DONE0 "))
            .await
            .unwrap();
        assert_eq!(
            found.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["done0"]
        );
        let r = RunResult {
            text: "找到的答案".into(),
            ..Default::default()
        };
        TaskService::set_result(&pool, &queued.id, &r)
            .await
            .unwrap();
        assert_eq!(
            TaskService::list_tasks(&pool, Some("答案"))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn completing_registers_files_in_output_dir() {
        let dir = std::env::temp_dir().join(format!("ab-out-{}", uuid::Uuid::new_v4()));
        let path = std::env::temp_dir().join(format!("agent_board_test_{}.db", uuid::Uuid::new_v4()));
        let pool = crate::db::init_db(path.to_str().unwrap()).await.unwrap();
        let t = new_task(&pool, "gen", None).await;
        sqlx::query("UPDATE tasks SET workdir = ? WHERE id = ?")
            .bind(dir.to_str().unwrap())
            .bind(&t.id)
            .execute(&pool)
            .await
            .unwrap();
        let claimed = TaskService::claim_task(&pool, "agy").await.unwrap().unwrap();
        let out = dir.join(crate::features::artifacts::output_dir(&t.id));
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("a.png"), b"x").unwrap();

        assert!(TaskService::complete_task(&pool, &t.id, &claimed.lease_id).await.unwrap());
        let list = crate::features::artifacts::ArtifactService::list_by_task(&pool, &t.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].file_path.ends_with("/a.png"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn concurrent_claims_never_hand_out_same_task() {
        let path =
            std::env::temp_dir().join(format!("agent_board_test_{}.db", uuid::Uuid::new_v4()));
        let pool = crate::db::init_db(path.to_str().unwrap()).await.unwrap();
        for i in 0..3 {
            new_task(&pool, &format!("t{i}"), None).await;
        }

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let pool = pool.clone();
                tokio::spawn(async move {
                    TaskService::claim_task(&pool, &format!("agent{i}"))
                        .await
                        .unwrap()
                })
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

    #[tokio::test]
    async fn followup_inherits_settings_session_and_worktree() {
        let pool = memory_pool().await;
        let parent = new_task(&pool, "寫功能", None).await;
        let opts = RunOptions {
            agent_id: Some("claude".into()),
            model: Some("haiku".into()),
            workdir: Some("/repo".into()),
            isolate: Some(true),
            ..Default::default()
        };
        let claimed = TaskService::claim_specific(&pool, &parent.id, "claude", &opts)
            .await
            .unwrap()
            .unwrap();
        // 執行中不能追加
        assert!(
            TaskService::create_followup(&pool, &parent.id, "再補測試")
                .await
                .unwrap()
                .is_none()
        );
        TaskService::set_session(&pool, &parent.id, "sess-1")
            .await
            .unwrap();
        TaskService::set_worktree(&pool, &parent.id, Some(("/wt/x", "agent-board/x", "abc")))
            .await
            .unwrap();
        TaskService::complete_task(&pool, &parent.id, &claimed.lease_id)
            .await
            .unwrap();

        let child = TaskService::create_followup(&pool, &parent.id, "再補測試")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(child.title, "↳ 寫功能");
        assert_eq!(child.status, "queued");
        assert_eq!(child.prompt, "再補測試");
        assert_eq!(child.parent_task_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(child.assigned_agent.as_deref(), Some("claude"));
        assert_eq!(child.model.as_deref(), Some("haiku"));
        assert_eq!(child.session_id.as_deref(), Some("sess-1"));
        assert_eq!(child.branch.as_deref(), Some("agent-board/x"));
        assert!(child.isolate);
        let parent = TaskService::get_task(&pool, &parent.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            parent.worktree_path, None,
            "worktree moves to the follow-up"
        );

        // 追加的追加不會疊加箭頭
        let claimed =
            TaskService::claim_specific(&pool, &child.id, "claude", &RunOptions::default())
                .await
                .unwrap()
                .unwrap();
        TaskService::fail_task(&pool, &child.id, &claimed.lease_id)
            .await
            .unwrap();
        let grandchild = TaskService::create_followup(&pool, &child.id, "修好它")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grandchild.title, "↳ 寫功能");
    }

    #[tokio::test]
    async fn stores_run_result() {
        let pool = memory_pool().await;
        let task = new_task(&pool, "a", None).await;
        let r = RunResult {
            text: "done".into(),
            cost_usd: Some(0.5),
            input_tokens: Some(10),
            output_tokens: Some(3),
            duration_ms: Some(1200),
            is_error: false,
        };
        TaskService::set_result(&pool, &task.id, &r).await.unwrap();
        let t = TaskService::get_task(&pool, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                t.result.as_deref(),
                t.cost_usd,
                t.input_tokens,
                t.output_tokens,
                t.duration_ms
            ),
            (Some("done"), Some(0.5), Some(10), Some(3), Some(1200))
        );

        // agy：逐步累加，結果事件只帶文字
        let other = new_task(&pool, "b", None).await;
        for _ in 0..2 {
            TaskService::add_usage(
                &pool,
                &other.id,
                &Usage {
                    input_tokens: 100,
                    output_tokens: 5,
                    duration_ms: 700,
                },
            )
            .await
            .unwrap();
        }
        let text_only = RunResult {
            text: "ok".into(),
            ..Default::default()
        };
        TaskService::set_result(&pool, &other.id, &text_only)
            .await
            .unwrap();
        let t = TaskService::get_task(&pool, &other.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                t.result.as_deref(),
                t.input_tokens,
                t.output_tokens,
                t.duration_ms
            ),
            (Some("ok"), Some(200), Some(10), Some(1400))
        );

        // 重新領取時清空上一次的結果
        let claimed = TaskService::claim_specific(&pool, &task.id, "agy", &RunOptions::default())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                claimed.task.result,
                claimed.task.cost_usd,
                claimed.task.input_tokens
            ),
            (None, None, None)
        );
    }

    #[tokio::test]
    async fn create_task_with_missing_dependency_is_rolled_back() {
        let pool = memory_pool().await;
        let res = TaskService::create_task(
            &pool,
            CreateTaskDto {
                title: "x".into(),
                prompt: "p".into(),
                dependency_ids: Some(vec!["no-such-task".into()]),
                ..Default::default()
            },
        )
        .await;
        assert!(res.is_err());
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}
