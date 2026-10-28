use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

/// 單一接收者視角的訊息；`is_read` 依查詢者計算（廣播訊息每個接收者各自已讀）
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentMessage {
    pub id: String,
    pub sender: String,
    pub receiver: String,
    pub payload: String,
    /// 訊息所屬任務，可為空
    pub task_id: Option<String>,
    pub is_read: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageDto {
    pub sender: String,
    /// Agent ID；`*` 代表廣播給除了發送者以外的所有人
    pub receiver: String,
    pub payload: String,
    pub task_id: Option<String>,
}

/// 預設最多回傳幾則訊息
pub const DEFAULT_MESSAGE_LIMIT: i64 = 100;

/// 收件匣條件：寄給自己的，或別人發出的廣播
const INBOX: &str = "(m.receiver = ?1 OR (m.receiver = '*' AND m.sender != ?1))";
const IS_READ: &str =
    "EXISTS (SELECT 1 FROM message_reads r WHERE r.message_id = m.id AND r.reader = ?1)";

pub struct MessageService;

impl MessageService {
    pub async fn send_message(
        pool: &Pool<Sqlite>,
        dto: SendMessageDto,
    ) -> Result<AgentMessage, sqlx::Error> {
        let msg = AgentMessage {
            id: uuid::Uuid::new_v4().to_string(),
            sender: dto.sender,
            receiver: dto.receiver,
            payload: dto.payload,
            task_id: dto.task_id.filter(|t| !t.is_empty()),
            is_read: false,
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query(
            r#"
            INSERT INTO agent_messages (id, sender, receiver, payload, task_id, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&msg.id)
        .bind(&msg.sender)
        .bind(&msg.receiver)
        .bind(&msg.payload)
        .bind(&msg.task_id)
        .bind(&msg.created_at)
        .execute(pool)
        .await?;

        Ok(msg)
    }

    /// 取得 `reader` 的收件匣（由舊到新）
    /// unread_only = true 時取最舊的未讀；否則取最近 `limit` 則
    pub async fn get_messages(
        pool: &Pool<Sqlite>,
        reader: &str,
        unread_only: bool,
        task_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AgentMessage>, sqlx::Error> {
        let unread = if unread_only {
            format!("AND NOT {IS_READ}")
        } else {
            String::new()
        };
        let order = if unread_only { "ASC" } else { "DESC" };
        let sql = format!(
            r#"
            SELECT * FROM (
                SELECT m.id, m.sender, m.receiver, m.payload, m.task_id, m.created_at, {IS_READ} AS is_read
                FROM agent_messages m
                WHERE {INBOX} AND (?2 IS NULL OR m.task_id = ?2) {unread}
                ORDER BY m.created_at {order}, m.rowid {order} LIMIT ?3
            ) ORDER BY created_at ASC
            "#
        );
        sqlx::query_as::<_, AgentMessage>(&sql)
            .bind(reader)
            .bind(task_id)
            .bind(limit.clamp(1, 1000))
            .fetch_all(pool)
            .await
    }

    /// 看板用：最近 `limit` 則全部訊息（由舊到新）；`is_read` 表示收件者本人是否已讀（廣播恆為 false）
    pub async fn recent(pool: &Pool<Sqlite>, limit: i64) -> Result<Vec<AgentMessage>, sqlx::Error> {
        sqlx::query_as::<_, AgentMessage>(
            r#"
            SELECT * FROM (
                SELECT m.id, m.sender, m.receiver, m.payload, m.task_id, m.created_at,
                       EXISTS (SELECT 1 FROM message_reads r WHERE r.message_id = m.id AND r.reader = m.receiver) AS is_read
                FROM agent_messages m
                ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?1
            ) ORDER BY created_at ASC
            "#,
        )
        .bind(limit.clamp(1, 1000))
        .fetch_all(pool)
        .await
    }

    /// 把訊息標記為 `reader` 已讀；ids 為 None 代表收件匣內全部。回傳實際新標記的則數
    pub async fn mark_read(
        pool: &Pool<Sqlite>,
        reader: &str,
        ids: Option<&[String]>,
    ) -> Result<u64, sqlx::Error> {
        let ids_json = ids.map(|ids| serde_json::to_string(ids).unwrap_or_default());
        let sql = format!(
            r#"
            INSERT OR IGNORE INTO message_reads (message_id, reader, read_at)
            SELECT m.id, ?1, ?3 FROM agent_messages m
            WHERE {INBOX} AND (?2 IS NULL OR m.id IN (SELECT value FROM json_each(?2)))
            "#
        );
        let rows = sqlx::query(&sql)
            .bind(reader)
            .bind(ids_json)
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await?
            .rows_affected();
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn memory_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    async fn send(pool: &Pool<Sqlite>, from: &str, to: &str, text: &str) -> AgentMessage {
        MessageService::send_message(
            pool,
            SendMessageDto {
                sender: from.into(),
                receiver: to.into(),
                payload: text.into(),
                task_id: None,
            },
        )
        .await
        .unwrap()
    }

    async fn inbox(pool: &Pool<Sqlite>, who: &str, unread: bool) -> Vec<String> {
        MessageService::get_messages(pool, who, unread, None, DEFAULT_MESSAGE_LIMIT)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.payload)
            .collect()
    }

    #[tokio::test]
    async fn direct_messages_reach_only_the_receiver() {
        let pool = memory_pool().await;
        send(&pool, "claude", "agy", "hi").await;
        assert_eq!(inbox(&pool, "agy", true).await, ["hi"]);
        assert!(inbox(&pool, "claude", true).await.is_empty());
        assert!(inbox(&pool, "other", true).await.is_empty());
    }

    #[tokio::test]
    async fn broadcast_is_read_per_recipient_and_skips_sender() {
        let pool = memory_pool().await;
        send(&pool, "claude", "*", "all hands").await;

        assert!(inbox(&pool, "claude", false).await.is_empty());
        assert_eq!(MessageService::mark_read(&pool, "agy", None).await.unwrap(), 1);
        assert!(inbox(&pool, "agy", true).await.is_empty());
        assert_eq!(inbox(&pool, "other", true).await, ["all hands"]);
        // 已讀的仍可在完整收件匣看到，且標示已讀
        let all = MessageService::get_messages(&pool, "agy", false, None, 10)
            .await
            .unwrap();
        assert!(all[0].is_read);
    }

    #[tokio::test]
    async fn recent_lists_everything_with_receiver_read_state() {
        let pool = memory_pool().await;
        send(&pool, "a", "b", "one").await;
        send(&pool, "a", "*", "two").await;
        MessageService::mark_read(&pool, "b", None).await.unwrap();

        let all = MessageService::recent(&pool, 10).await.unwrap();
        let state: Vec<_> = all.iter().map(|m| (m.payload.as_str(), m.is_read)).collect();
        assert_eq!(state, [("one", true), ("two", false)]);
    }

    #[tokio::test]
    async fn mark_read_by_ids_and_task_filter() {
        let pool = memory_pool().await;
        let first = send(&pool, "a", "b", "one").await;
        send(&pool, "a", "b", "two").await;
        let dto = SendMessageDto {
            sender: "a".into(),
            receiver: "b".into(),
            payload: "three".into(),
            task_id: Some("t1".into()),
        };
        MessageService::send_message(&pool, dto).await.unwrap();

        MessageService::mark_read(&pool, "b", Some(&[first.id.clone()]))
            .await
            .unwrap();
        assert_eq!(inbox(&pool, "b", true).await, ["two", "three"]);
        let for_task = MessageService::get_messages(&pool, "b", false, Some("t1"), 10)
            .await
            .unwrap();
        assert_eq!(for_task.len(), 1);
        // 重複標記不會重複計數
        assert_eq!(
            MessageService::mark_read(&pool, "b", Some(&[first.id]))
                .await
                .unwrap(),
            0
        );
    }
}
