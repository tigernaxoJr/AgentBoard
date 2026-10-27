use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::fs;
use std::path::Path;

pub async fn init_db(db_path: &str) -> Result<Pool<Sqlite>, sqlx::Error> {
    if let Some(parent) = Path::new(db_path).parent() {
        if !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    }

    let connection_url = if db_path == ":memory:" {
        "sqlite::memory:".to_string()
    } else {
        if !Path::new(db_path).exists() {
            let _ = fs::File::create(db_path);
        }
        format!("sqlite://{}", db_path)
    };

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(&connection_url)
        .await?;

    // Enable WAL mode & foreign keys
    sqlx::query("PRAGMA journal_mode = WAL;").execute(&pool).await?;
    sqlx::query("PRAGMA foreign_keys = ON;").execute(&pool).await?;
    sqlx::query("PRAGMA busy_timeout = 5000;").execute(&pool).await?;

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
            depends_on_task_id TEXT,
            last_heartbeat_at TEXT,
            lease_id TEXT,
            attempts INTEGER NOT NULL DEFAULT 0,
            priority INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (depends_on_task_id) REFERENCES tasks(id)
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
            read_status INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
        CREATE INDEX IF NOT EXISTS idx_tasks_assigned_agent ON tasks(assigned_agent);
        CREATE INDEX IF NOT EXISTS idx_task_logs_task_id ON task_logs(task_id);
        CREATE INDEX IF NOT EXISTS idx_artifacts_task_id ON artifacts(task_id);
        CREATE INDEX IF NOT EXISTS idx_agent_messages_receiver ON agent_messages(receiver, read_status);
        "#,
    )
    .execute(pool)
    .await?;

    for (column, ddl) in [
        ("lease_id", "lease_id TEXT"),
        ("attempts", "attempts INTEGER NOT NULL DEFAULT 0"),
        ("priority", "priority INTEGER NOT NULL DEFAULT 0"),
    ] {
        let exists: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM pragma_table_info('tasks') WHERE name = ?")
            .bind(column)
            .fetch_one(pool)
            .await?;
        if !exists {
            sqlx::query(&format!("ALTER TABLE tasks ADD COLUMN {ddl}")).execute(pool).await?;
        }
    }

    Ok(())
}
