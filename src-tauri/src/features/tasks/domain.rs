use chrono::Utc;
use serde::{Deserialize, Serialize};

/// 任務資料列。狀態轉移與租約規則以原子 SQL 實作於 service.rs，
/// 避免「先讀再判斷再寫」在多個 Agent 並發時出現競態
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub status: String,
    pub assigned_agent: Option<String>,
    pub creator: String,
    /// 依賴 ID 清單
    #[sqlx(skip)]
    pub dependency_ids: Vec<String>,
    /// 追加指示建立的任務指向原任務（不是依賴：原任務失敗也可以追加）
    pub parent_task_id: Option<String>,
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
    /// true = 在獨立的 git worktree 中執行（workdir 必須在 git repo 內）
    pub isolate: bool,
    /// 隔離執行時建立的 worktree、分支與分支起點
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    /// Agent 的工作階段 ID（claude session_id / agy conversation_id）；有值時下次執行會接續該工作階段
    pub session_id: Option<String>,
    /// 最後一次執行的結果文字、費用（USD，agy 不提供）、token 數與耗時
    pub result: Option<String>,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl Task {
    pub fn new(
        title: String,
        prompt: String,
        creator: String,
        assigned_agent: Option<String>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            prompt,
            status: "queued".to_string(),
            assigned_agent,
            creator,
            dependency_ids: Vec::new(),
            parent_task_id: None,
            last_heartbeat_at: None,
            lease_id: None,
            attempts: 0,
            priority: 0,
            model: None,
            workdir: None,
            auto_approve: false,
            isolate: false,
            worktree_path: None,
            branch: None,
            base_commit: None,
            session_id: None,
            result: None,
            cost_usd: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}
