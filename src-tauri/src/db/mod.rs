use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::time::Duration;

pub async fn init_db(db_path: &str) -> Result<Pool<Sqlite>, sqlx::Error> {
    if let Some(parent) = Path::new(db_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    // PRAGMA 是每條連線各自的設定，放在連線選項裡才會套用到連線池的每一條連線
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await?;

    migrate(&pool).await?;
    Ok(pool)
}

/// 建立資料表，並為舊資料庫補上後來新增的欄位
pub async fn migrate(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            prompt TEXT NOT NULL,
            status TEXT NOT NULL,
            assigned_agent TEXT,
            creator TEXT NOT NULL,
            last_heartbeat_at TEXT,
            lease_id TEXT,
            attempts INTEGER NOT NULL DEFAULT 0,
            priority INTEGER NOT NULL DEFAULT 0,
            model TEXT,
            workdir TEXT,
            auto_approve INTEGER NOT NULL DEFAULT 0,
            isolate INTEGER NOT NULL DEFAULT 0,
            worktree_path TEXT,
            branch TEXT,
            base_commit TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS artifacts (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            file_path TEXT NOT NULL,
            summary TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (task_id) REFERENCES tasks(id)
        );

        CREATE TABLE IF NOT EXISTS task_logs (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            level TEXT NOT NULL,
            message TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (task_id) REFERENCES tasks(id)
        );

        CREATE TABLE IF NOT EXISTS agent_messages (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            receiver TEXT NOT NULL,
            payload TEXT NOT NULL,
            task_id TEXT,
            created_at TEXT NOT NULL
        );

        -- 每個接收者各自的已讀紀錄（廣播訊息才不會被一個人讀過就對其他人消失）
        CREATE TABLE IF NOT EXISTS message_reads (
            message_id TEXT NOT NULL,
            reader TEXT NOT NULL,
            read_at TEXT NOT NULL,
            PRIMARY KEY (message_id, reader)
        );

        CREATE TABLE IF NOT EXISTS task_templates (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            title TEXT NOT NULL,
            prompt TEXT NOT NULL,
            assigned_agent TEXT,
            model TEXT,
            workdir TEXT,
            auto_approve INTEGER NOT NULL DEFAULT 1,
            isolate INTEGER NOT NULL DEFAULT 0,
            priority INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS task_dependencies (
            task_id TEXT NOT NULL,
            depends_on_task_id TEXT NOT NULL,
            PRIMARY KEY (task_id, depends_on_task_id),
            FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
            FOREIGN KEY (depends_on_task_id) REFERENCES tasks(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_task_dependencies_dep ON task_dependencies(depends_on_task_id);

        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
        CREATE INDEX IF NOT EXISTS idx_tasks_assigned_agent ON tasks(assigned_agent);
        CREATE INDEX IF NOT EXISTS idx_task_logs_task_time ON task_logs(task_id, created_at);
        DROP INDEX IF EXISTS idx_task_logs_task_id;
        CREATE INDEX IF NOT EXISTS idx_artifacts_task_id ON artifacts(task_id);
        DROP INDEX IF EXISTS idx_agent_messages_receiver;
        CREATE INDEX IF NOT EXISTS idx_agent_messages_receiver ON agent_messages(receiver, created_at);
        "#,
    )
    .execute(pool)
    .await?;

    // 舊資料庫的 agent_messages 仍帶有已不使用的 read_status 欄位（有預設值，INSERT 不必提供）
    let has_task_id: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('agent_messages') WHERE name = 'task_id'",
    )
    .fetch_one(pool)
    .await?;
    if !has_task_id {
        sqlx::query("ALTER TABLE agent_messages ADD COLUMN task_id TEXT")
            .execute(pool)
            .await?;
    }

    for (column, ddl) in [
        ("lease_id", "lease_id TEXT"),
        ("attempts", "attempts INTEGER NOT NULL DEFAULT 0"),
        ("priority", "priority INTEGER NOT NULL DEFAULT 0"),
        ("model", "model TEXT"),
        ("workdir", "workdir TEXT"),
        ("auto_approve", "auto_approve INTEGER NOT NULL DEFAULT 0"),
        ("isolate", "isolate INTEGER NOT NULL DEFAULT 0"),
        ("worktree_path", "worktree_path TEXT"),
        ("branch", "branch TEXT"),
        ("base_commit", "base_commit TEXT"),
        ("session_id", "session_id TEXT"),
        ("parent_task_id", "parent_task_id TEXT"),
        ("result", "result TEXT"),
        ("cost_usd", "cost_usd REAL"),
        ("input_tokens", "input_tokens INTEGER"),
        ("output_tokens", "output_tokens INTEGER"),
        ("duration_ms", "duration_ms INTEGER"),
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('tasks') WHERE name = ?",
        )
        .bind(column)
        .fetch_one(pool)
        .await?;
        if !exists {
            sqlx::query(&format!("ALTER TABLE tasks ADD COLUMN {ddl}"))
                .execute(pool)
                .await?;
        }
    }

    Ok(())
}
