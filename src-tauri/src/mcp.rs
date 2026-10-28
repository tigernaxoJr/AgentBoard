use std::path::PathBuf;
use std::sync::Arc;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const APP_IDENTIFIER: &str = "com.naxo.agent-board";
const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:18833";
const DEFAULT_LIMIT: usize = 10;
const DEFAULT_MAX_CHARS: usize = 1500;
const DEFAULT_WAIT_SECS: u64 = 60;
const MAX_WAIT_SECS: u64 = 300;
const WAIT_POLL_SECS: u64 = 2;

fn get_default_token_file() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_IDENTIFIER)
        .join("api_token")
}

fn resolve_token() -> String {
    if let Ok(token) = std::env::var("AGENT_BOARD_TOKEN") {
        let t = token.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    let token_file = get_default_token_file();
    if let Ok(token) = std::fs::read_to_string(token_file) {
        let t = token.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    String::new()
}

#[derive(Clone)]
pub struct BoardHttpClient {
    server_url: String,
    token: String,
}

impl BoardHttpClient {
    pub fn new() -> Self {
        let server_url = std::env::var("AGENT_BOARD_SERVER")
            .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let token = resolve_token();
        Self { server_url, token }
    }

    pub fn get(&self, path: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.server_url, path);
        let mut req = ureq::get(&url);
        if !self.token.is_empty() {
            req = req.header("Authorization", &format!("Bearer {}", self.token));
        }
        self.handle_response(req.call(), &url)
    }

    pub fn post(&self, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.server_url, path);
        let mut req = ureq::post(&url);
        if !self.token.is_empty() {
            req = req.header("Authorization", &format!("Bearer {}", self.token));
        }
        let resp = if let Some(payload) = body {
            req.send_json(payload)
        } else {
            req.send_empty()
        };
        self.handle_response(resp, &url)
    }

    fn handle_response(
        &self,
        resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
        url: &str,
    ) -> Result<serde_json::Value, String> {
        match resp {
            Ok(mut res) => {
                let status = res.status();
                let text = res
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| format!("Failed to read response body: {e}"))?;
                if text.trim().is_empty() {
                    return Ok(serde_json::json!({ "status": status.as_u16() }));
                }
                serde_json::from_str::<serde_json::Value>(&text)
                    .or_else(|_| Ok(serde_json::Value::String(text)))
            }
            Err(ureq::Error::StatusCode(code)) => {
                Err(format!("HTTP Error {code} from Agent Board at {url}"))
            }
            Err(e) => {
                Err(format!(
                    "Agent Board is not running or unreachable ({e}). Please launch the Agent Board desktop app."
                ))
            }
        }
    }
}

/// 查詢字串用的百分比編碼（只保留 RFC 3986 的非保留字元）
fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn clip(text: &str, limit: usize) -> String {
    if limit == 0 || text.chars().count() <= limit {
        return text.to_string();
    }
    let count = text.chars().count();
    let head = limit * 2 / 3;
    let tail = limit.saturating_sub(head);
    let head_str: String = text.chars().take(head).collect();
    let tail_str: String = text.chars().skip(count.saturating_sub(tail)).collect();
    format!("{head_str}\n…（省略 {} 字，傳 full=true 或調高 max_chars 取得完整內容）…\n{tail_str}", count - limit)
}

fn slim_task(mut val: serde_json::Value, max_chars: usize) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = val {
        let drop_fields = [
            "last_heartbeat_at",
            "lease_id",
            "base_commit",
            "worktree_path",
            "updated_at",
        ];
        for f in drop_fields {
            map.remove(f);
        }
        for k in ["prompt", "result"] {
            if let Some(serde_json::Value::String(s)) = map.get(k) {
                let clipped = clip(s, max_chars);
                map.insert(k.to_string(), serde_json::Value::String(clipped));
            }
        }
    }
    val
}

// Tool Parameter Structs
#[derive(Deserialize, JsonSchema)]
pub struct ListTasksParams {
    /// 搜尋標題/Prompt/ID/結果
    pub query: Option<String>,
    /// 狀態過濾: pending_approval, queued, running, completed, failed
    pub status: Option<String>,
    /// 最多回傳幾筆，預設 10
    pub limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetTaskParams {
    /// 任務 ID
    pub task_id: String,
    /// prompt/result 各自長度上限，預設 1500
    pub max_chars: Option<usize>,
    /// true = 不截斷且包含 prompt
    pub full: Option<bool>,
    /// 附上 prompt（預設不附）
    pub include_prompt: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitTaskParams {
    /// 任務 ID
    pub task_id: String,
    /// 最多等幾秒，預設 60、上限 300（超過 MCP 用戶端逾時會被中斷）
    pub timeout_secs: Option<u64>,
    /// prompt/result 各自長度上限，預設 1500
    pub max_chars: Option<usize>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct CreateTaskParams {
    /// 任務標題
    pub title: String,
    /// 提示詞；可用 {{dep.result}} 引用前置結果
    pub prompt: String,
    /// 指定執行的 Agent（例如 'agy' 或 'claude'，不填代表未指派，任何 Agent 可搶單）
    pub assigned_agent: Option<String>,
    /// 建立者標記（預設 'mcp'）
    pub creator: Option<String>,
    /// 前置任務 ID 陣列，全部完成才執行
    pub dependency_ids: Option<Vec<String>>,
    /// 優先級（越大越先被領取，預設 0）
    pub priority: Option<i32>,
    /// 指定模型名稱（例如 'sonnet', 'haiku', 'pro', 'flash'）
    pub model: Option<String>,
    /// 任務執行時的工作目錄路徑
    pub workdir: Option<String>,
    /// 略過權限確認
    pub auto_approve: Option<bool>,
    /// 用獨立 git worktree 隔離
    pub isolate: Option<bool>,
    /// true = 任務先等人類在看板核准才會排入佇列（建議：由 Agent 再派發、會產生副作用的任務）
    pub requires_approval: Option<bool>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DispatchTaskParams {
    /// 任務 ID
    pub task_id: String,
    /// 指定 Agent（例如 'agy', 'claude'）
    pub agent: Option<String>,
    /// 指定模型
    pub model: Option<String>,
    /// 指定工作目錄
    pub workdir: Option<String>,
    /// 是否略過授權
    pub auto_approve: Option<bool>,
    /// 是否以 worktree 隔離
    pub isolate: Option<bool>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct SpawnTaskParams {
    /// 任務 ID
    pub task_id: String,
    /// 指定 Agent（例如 'agy', 'claude'）
    pub agent: Option<String>,
    /// 指定模型
    pub model: Option<String>,
    /// 指定工作目錄
    pub workdir: Option<String>,
    /// 是否略過授權
    pub auto_approve: Option<bool>,
    /// 是否以 worktree 隔離
    pub isolate: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CancelTaskParams {
    /// 任務 ID
    pub task_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RequeueTaskParams {
    /// 任務 ID
    pub task_id: String,
    /// true = 捨棄原 session 從頭跑
    pub fresh: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FollowupTaskParams {
    /// 原任務 ID
    pub task_id: String,
    /// 追加的指令內容
    pub prompt: String,
    /// 建立者標記（預設 'mcp'）
    pub creator: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddArtifactParams {
    /// 任務 ID
    pub task_id: String,
    /// 登記者 Agent ID
    pub agent_id: String,
    /// 成果檔案絕對或相對路徑
    pub file_path: String,
    /// 成果簡述
    pub summary: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListArtifactsParams {
    /// 任務 ID
    pub task_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// 發送者 ID
    pub sender: String,
    /// 接收者 ID
    pub receiver: String,
    /// 訊息文字或 JSON 內容
    pub payload: String,
    /// 關聯的任務 ID（可選）
    pub task_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadMessagesParams {
    /// 收件者 ID（自己的 Agent ID）
    pub receiver: String,
    /// true（預設）= 只取未讀；false = 取最近的全部訊息
    pub unread_only: Option<bool>,
    /// 只取指定任務的訊息
    pub task_id: Option<String>,
    /// 最多回傳幾則，預設 20
    pub limit: Option<usize>,
    /// 取回後立即標為已讀，預設 true
    pub mark_read: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct EmptyParams {}

#[derive(Clone)]
pub struct AgentBoardMcpServer {
    client: Arc<BoardHttpClient>,
    tool_router: ToolRouter<Self>,
}

impl AgentBoardMcpServer {
    pub fn new() -> Self {
        Self {
            client: Arc::new(BoardHttpClient::new()),
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for AgentBoardMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgentBoardMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new("agent-board", "0.1.0"))
            .with_instructions("Agent Board MCP server: manage tasks, agents, and artifacts.")
    }
}

fn to_call_result(res: Result<serde_json::Value, String>) -> CallToolResult {
    match res {
        Ok(val) => {
            let text = serde_json::to_string(&val).unwrap_or_else(|_| val.to_string());
            CallToolResult::success(vec![ContentBlock::text(text)])
        }
        Err(err) => CallToolResult::error(vec![ContentBlock::text(err)]),
    }
}

#[tool_router(router = tool_router)]
impl AgentBoardMcpServer {
    #[tool(name = "list_tasks", description = "列出任務摘要（不含 prompt/result 全文，需要細節請用 get_task）")]
    async fn list_tasks(&self, params: Parameters<ListTasksParams>) -> CallToolResult {
        let p = params.0;
        let mut path = "/api/tasks".to_string();
        if let Some(ref q) = p.query {
            let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
            path = format!("{path}?q={encoded}");
        }

        let res = self.client.get(&path).map(|val| {
            if let serde_json::Value::Array(tasks) = val {
                let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
                let filtered: Vec<serde_json::Value> = tasks
                    .into_iter()
                    .filter(|t| {
                        if let Some(ref s) = p.status {
                            t.get("status").and_then(|v| v.as_str()) == Some(s)
                        } else {
                            true
                        }
                    })
                    .collect();

                let total = filtered.len();
                let list_fields = ["id", "title", "status", "assigned_agent", "priority", "dependency_ids", "parent_task_id", "cost_usd"];
                let mut shown: Vec<serde_json::Value> = filtered
                    .into_iter()
                    .take(limit)
                    .map(|t| {
                        let mut map = serde_json::Map::new();
                        if let serde_json::Value::Object(t_map) = t {
                            for f in list_fields {
                                if let Some(v) = t_map.get(f) {
                                    if !v.is_null() {
                                        map.insert(f.to_string(), v.clone());
                                    }
                                }
                            }
                            if let Some(serde_json::Value::String(res_str)) = t_map.get("result") {
                                map.insert("result_preview".to_string(), serde_json::Value::String(clip(res_str, 120)));
                            }
                        }
                        serde_json::Value::Object(map)
                    })
                    .collect();

                if total > limit {
                    shown.push(serde_json::json!({
                        "note": format!("共 {total} 筆，僅顯示前 {limit} 筆；請用 query / status / limit 縮小或擴大範圍")
                    }));
                }
                serde_json::Value::Array(shown)
            } else {
                val
            }
        });

        to_call_result(res)
    }

    #[tool(name = "get_task", description = "取得任務詳情；prompt/result 預設截斷至 1500 字且不含 prompt")]
    async fn get_task(&self, params: Parameters<GetTaskParams>) -> CallToolResult {
        let p = params.0;
        let res = self.client.get(&format!("/api/tasks/{}", p.task_id)).map(|mut val| {
            if p.full.unwrap_or(false) {
                return val;
            }
            if !p.include_prompt.unwrap_or(false) {
                if let serde_json::Value::Object(ref mut map) = val {
                    map.remove("prompt");
                }
            }
            slim_task(val, p.max_chars.unwrap_or(DEFAULT_MAX_CHARS))
        });
        to_call_result(res)
    }

    #[tool(name = "wait_task", description = "等待任務結束（completed / failed）後回傳結果；逾時則回傳目前狀態並標示 timed_out，可再呼叫一次繼續等")]
    async fn wait_task(&self, params: Parameters<WaitTaskParams>) -> CallToolResult {
        let p = params.0;
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(p.timeout_secs.unwrap_or(DEFAULT_WAIT_SECS).min(MAX_WAIT_SECS));
        loop {
            let mut task = match self.client.get(&format!("/api/tasks/{}", p.task_id)) {
                Ok(task) => task,
                Err(e) => return to_call_result(Err(e)),
            };
            let finished = matches!(task["status"].as_str(), Some("completed" | "failed"));
            let timed_out = !finished && std::time::Instant::now() >= deadline;
            if finished || timed_out {
                if let serde_json::Value::Object(ref mut map) = task {
                    map.remove("prompt");
                    map.insert("timed_out".into(), timed_out.into());
                }
                return to_call_result(Ok(slim_task(task, p.max_chars.unwrap_or(DEFAULT_MAX_CHARS))));
            }
            tokio::time::sleep(std::time::Duration::from_secs(WAIT_POLL_SECS)).await;
        }
    }

    #[tool(name = "create_task", description = "建立看板任務")]
    async fn create_task(&self, params: Parameters<CreateTaskParams>) -> CallToolResult {
        let payload = serde_json::to_value(&params.0).unwrap_or(serde_json::Value::Null);
        let res = self.client.post("/api/tasks", Some(payload)).map(|val| {
            slim_task(val, 200)
        });
        to_call_result(res)
    }

    #[tool(name = "dispatch_task", description = "排入佇列等 Runner 領取")]
    async fn dispatch_task(&self, params: Parameters<DispatchTaskParams>) -> CallToolResult {
        let p = params.0;
        let task_id = p.task_id.clone();
        let payload = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
        let res = self.client.post(&format!("/api/tasks/{task_id}/dispatch"), Some(payload)).map(|val| {
            slim_task(val, 200)
        });
        to_call_result(res)
    }

    #[tool(name = "spawn_task", description = "由看板立即啟動本機子程序執行")]
    async fn spawn_task(&self, params: Parameters<SpawnTaskParams>) -> CallToolResult {
        let p = params.0;
        let task_id = p.task_id.clone();
        let payload = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
        let res = self.client.post(&format!("/api/tasks/{task_id}/spawn"), Some(payload)).map(|val| {
            slim_task(val, 200)
        });
        to_call_result(res)
    }

    #[tool(name = "cancel_task", description = "取消正在執行或佇列中的任務")]
    async fn cancel_task(&self, params: Parameters<CancelTaskParams>) -> CallToolResult {
        let p = params.0;
        let res = self.client.post(&format!("/api/tasks/{}/cancel", p.task_id), None);
        to_call_result(res)
    }

    #[tool(name = "requeue_task", description = "重排失敗任務")]
    async fn requeue_task(&self, params: Parameters<RequeueTaskParams>) -> CallToolResult {
        let p = params.0;
        let payload = serde_json::json!({ "fresh": p.fresh.unwrap_or(false) });
        let res = self.client.post(&format!("/api/tasks/{}/requeue", p.task_id), Some(payload));
        to_call_result(res)
    }

    #[tool(name = "followup_task", description = "對已結束任務追加指示（沿用原 session，建立新任務）")]
    async fn followup_task(&self, params: Parameters<FollowupTaskParams>) -> CallToolResult {
        let p = params.0;
        let payload = serde_json::json!({
            "prompt": p.prompt,
            "creator": p.creator.unwrap_or_else(|| "mcp".to_string())
        });
        let res = self.client.post(&format!("/api/tasks/{}/followup", p.task_id), Some(payload));
        to_call_result(res)
    }

    #[tool(name = "add_artifact", description = "登記任務所產生的成果檔案（Artifact）")]
    async fn add_artifact(&self, params: Parameters<AddArtifactParams>) -> CallToolResult {
        let p = params.0;
        let payload = serde_json::json!({
            "agent_id": p.agent_id,
            "file_path": p.file_path,
            "summary": p.summary
        });
        let res = self.client.post(&format!("/api/tasks/{}/artifacts", p.task_id), Some(payload));
        to_call_result(res)
    }

    #[tool(name = "list_artifacts", description = "列出指定任務登記的成果檔案清單")]
    async fn list_artifacts(&self, params: Parameters<ListArtifactsParams>) -> CallToolResult {
        let p = params.0;
        let res = self.client.get(&format!("/api/tasks/{}/artifacts", p.task_id));
        to_call_result(res)
    }

    #[tool(name = "list_runners", description = "列出目前在線的背景 Runner 及其狀態")]
    async fn list_runners(&self, _params: Parameters<EmptyParams>) -> CallToolResult {
        let res = self.client.get("/api/runners");
        to_call_result(res)
    }

    #[tool(name = "list_agents", description = "列出系統支援的 Agent 類型及可用模型")]
    async fn list_agents(&self, _params: Parameters<EmptyParams>) -> CallToolResult {
        let res = self.client.get("/api/agents");
        to_call_result(res)
    }

    #[tool(name = "send_message", description = "發送 Agent 間的內部訊息；receiver 填 '*' 代表廣播給其他所有 Agent")]
    async fn send_message(&self, params: Parameters<SendMessageParams>) -> CallToolResult {
        let p = params.0;
        let payload = serde_json::json!({
            "sender": p.sender,
            "receiver": p.receiver,
            "payload": p.payload,
            "task_id": p.task_id
        });
        let res = self.client.post("/api/messages", Some(payload));
        to_call_result(res)
    }

    #[tool(name = "read_messages", description = "讀取自己的收件匣（含別人的廣播）；預設只取未讀並標為已讀")]
    async fn read_messages(&self, params: Parameters<ReadMessagesParams>) -> CallToolResult {
        let p = params.0;
        let unread_only = p.unread_only.unwrap_or(true);
        let mut query = format!(
            "/api/messages?receiver={}&unread={}&limit={}",
            url_encode(&p.receiver),
            unread_only,
            p.limit.unwrap_or(20)
        );
        if let Some(task_id) = p.task_id.as_deref().filter(|t| !t.is_empty()) {
            query.push_str(&format!("&task_id={}", url_encode(task_id)));
        }
        let messages = match self.client.get(&query) {
            Ok(val) => val,
            Err(e) => return to_call_result(Err(e)),
        };

        if p.mark_read.unwrap_or(true) {
            let ids: Vec<&str> = messages
                .as_array()
                .map(|list| list.iter().filter_map(|m| m["id"].as_str()).collect())
                .unwrap_or_default();
            if !ids.is_empty() {
                let body = serde_json::json!({ "reader": p.receiver, "ids": ids });
                if let Err(e) = self.client.post("/api/messages/read", Some(body)) {
                    return to_call_result(Err(e));
                }
            }
        }
        to_call_result(Ok(messages))
    }
}

pub async fn run_mcp_stdio() -> Result<(), Box<dyn std::error::Error>> {
    let server = AgentBoardMcpServer::new();
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
