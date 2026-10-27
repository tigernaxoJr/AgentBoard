use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentMessage {
    pub id: String,
    pub sender: String,
    pub receiver: String,
    pub payload: String,
    pub read_status: i64,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageDto {
    pub sender: String,
    pub receiver: String,
    pub payload: String,
}

pub struct MessageService;

impl MessageService {
    pub async fn send_message(pool: &Pool<Sqlite>, dto: SendMessageDto) -> Result<AgentMessage, sqlx::Error> {
        let msg = AgentMessage {
            id: uuid::Uuid::new_v4().to_string(),
            sender: dto.sender,
            receiver: dto.receiver,
            payload: dto.payload,
            read_status: 0,
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query(
            r#"
            INSERT INTO agent_messages (id, sender, receiver, payload, read_status, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&msg.id)
        .bind(&msg.sender)
        .bind(&msg.receiver)
        .bind(&msg.payload)
        .bind(&msg.read_status)
        .bind(&msg.created_at)
        .execute(pool)
        .await?;

        Ok(msg)
    }

    pub async fn get_messages(
        pool: &Pool<Sqlite>,
        receiver: &str,
        unread_only: bool,
    ) -> Result<Vec<AgentMessage>, sqlx::Error> {
        if unread_only {
            let messages = sqlx::query_as::<_, AgentMessage>(
                "SELECT * FROM agent_messages WHERE (receiver = ? OR receiver = '*') AND read_status = 0 ORDER BY created_at ASC"
            )
            .bind(receiver)
            .fetch_all(pool)
            .await?;
            Ok(messages)
        } else {
            let messages = sqlx::query_as::<_, AgentMessage>(
                "SELECT * FROM agent_messages WHERE receiver = ? OR receiver = '*' ORDER BY created_at ASC"
            )
            .bind(receiver)
            .fetch_all(pool)
            .await?;
            Ok(messages)
        }
    }

    pub async fn mark_as_read(pool: &Pool<Sqlite>, id: &str) -> Result<bool, sqlx::Error> {
        let rows = sqlx::query("UPDATE agent_messages SET read_status = 1 WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
        Ok(rows > 0)
    }
}
