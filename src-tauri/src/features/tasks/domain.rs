use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Queued => "queued",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(TaskStatus::Queued),
            "running" => Some(TaskStatus::Running),
            "completed" => Some(TaskStatus::Completed),
            "failed" => Some(TaskStatus::Failed),
            _ => None,
        }
    }
}

/// 輕量領域模型：TaskAggregate
/// 管理任務的狀態轉移規則與租約（Lease / Heartbeat）邏輯
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub status: String,
    pub assigned_agent: Option<String>,
    pub creator: String,
    pub depends_on_task_id: Option<String>,
    pub last_heartbeat_at: Option<String>,
    /// 每次領取產生的租約 ID，只回傳給領取者，用來驗證 heartbeat / complete / fail
    #[serde(skip_serializing)]
    pub lease_id: Option<String>,
    /// 已被領取的次數（逾時重試達上限即標記 failed）
    pub attempts: i64,
    /// 派發優先序，數字越大越先被領取
    pub priority: i64,
    /// 指定模型，None 代表 Agent 預設
    pub model: Option<String>,
    /// 執行時的工作目錄
    pub workdir: Option<String>,
    /// true = 完整權限（略過所有權限詢問）
    pub auto_approve: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl Task {
    pub fn new(
        title: String,
        prompt: String,
        creator: String,
        assigned_agent: Option<String>,
        depends_on_task_id: Option<String>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            prompt,
            status: TaskStatus::Queued.as_str().to_string(),
            assigned_agent,
            creator,
            depends_on_task_id,
            last_heartbeat_at: None,
            lease_id: None,
            attempts: 0,
            priority: 0,
            model: None,
            workdir: None,
            auto_approve: false,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// 領取任務規則檢查
    pub fn can_claim(&self, agent_id: &str) -> bool {
        if self.status != TaskStatus::Queued.as_str() {
            return false;
        }
        match &self.assigned_agent {
            Some(assigned) => assigned == agent_id,
            None => true,
        }
    }

    /// 判斷心跳是否逾時（秒數）
    pub fn is_heartbeat_expired(&self, timeout_seconds: i64) -> bool {
        if self.status != TaskStatus::Running.as_str() {
            return false;
        }
        if let Some(ref hb_str) = self.last_heartbeat_at {
            if let Ok(hb_time) = chrono::DateTime::parse_from_rfc3339(hb_str) {
                let diff = Utc::now().signed_duration_since(hb_time.with_timezone(&Utc));
                return diff.num_seconds() > timeout_seconds;
            }
        }
        // 若處於 running 狀態卻從未回報過心跳，且距離 updated_at 超過超時時間也視為逾時
        if let Ok(up_time) = chrono::DateTime::parse_from_rfc3339(&self.updated_at) {
            let diff = Utc::now().signed_duration_since(up_time.with_timezone(&Utc));
            return diff.num_seconds() > timeout_seconds;
        }
        false
    }
}
