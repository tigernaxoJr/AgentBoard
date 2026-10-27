pub mod stream;

use crate::features::tasks::{AddTaskLogDto, Task, TaskService};
use crate::server::BoardEvent;
use serde::Serialize;
use sqlx::{Pool, Sqlite};
use stream::Parsed;
use tokio::sync::broadcast;

/// 支援的 Agent CLI 與模型預設清單；方案 A（Spawner）與方案 B（Runner）都由這裡組出 argv
#[derive(Debug, Serialize)]
pub struct AgentDef {
    pub id: &'static str,
    pub label: &'static str,
    pub default_model: &'static str,
    pub models: &'static [&'static str],
}

pub const AGENTS: &[AgentDef] = &[
    AgentDef {
        id: "agy",
        label: "Antigravity CLI",
        default_model: "gemini-3.8-flash-low",
        models: &[
            "gemini-3.8-flash-low",
            "gemini-3.8-flash-medium",
            "gemini-4-argon-medium",
            "gemini-4-argon-high",
            "claude-sonnet-5-5-medium",
            "claude-opus-5-5-high",
            "gpt-oss-120b-medium",
        ],
    },
    AgentDef {
        id: "claude",
        label: "Claude Code",
        default_model: "sonnet",
        models: &["sonnet", "opus", "haiku", "fable"],
    },
];

/// 依任務設定組出 argv；不支援的 Agent 回傳 None
/// 參數一律放在 prompt 前面（agy 用 Go flag 解析，遇到第一個非 flag 參數就停止），
/// 且不經過 shell，prompt 內容不會被當成指令解讀
pub fn command_for(agent_id: &str, task: &Task) -> Option<Vec<String>> {
    let mut argv = vec![agent_id.to_string(), "--output-format".into(), "stream-json".into()];
    if agent_id == "claude" {
        // claude 的 -p 搭配 stream-json 必須加 --verbose
        argv.push("--verbose".into());
    }
    // 有工作階段 ID（追加指示、或重試先前中斷的執行）時接續同一個對話
    if let Some(session) = task.session_id.as_deref() {
        let flag = if agent_id == "claude" { "--resume" } else { "--conversation" };
        argv.extend([flag.into(), session.into()]);
    }
    if let Some(model) = task.model.as_deref().filter(|m| !m.is_empty()) {
        argv.extend(["--model".into(), model.into()]);
    }
    match (agent_id, task.auto_approve) {
        ("agy", false) => argv.extend(["--mode".into(), "accept-edits".into()]),
        ("agy", true) => argv.push("--dangerously-skip-permissions".into()),
        ("claude", false) => argv.extend(["--permission-mode".into(), "acceptEdits".into()]),
        ("claude", true) => argv.extend(["--permission-mode".into(), "bypassPermissions".into()]),
        _ => return None,
    }
    argv.extend(["-p".into(), task.prompt.clone()]);
    Some(argv)
}

/// 處理 Agent 輸出的一行：寫日誌、記錄工作階段與結果，並廣播日誌事件
pub async fn ingest_output(pool: &Pool<Sqlite>, event_tx: &broadcast::Sender<BoardEvent>, task_id: &str, agent: &str, line: &str) {
    for parsed in stream::parse_line(line) {
        match parsed {
            Parsed::Log(level, message) => {
                let dto = AddTaskLogDto { agent_id: agent.into(), level: Some(level.into()), message };
                if let Ok(log) = TaskService::add_log(pool, task_id, dto).await {
                    let payload = serde_json::to_value(&log).unwrap_or_default();
                    let _ = event_tx.send(BoardEvent { event_type: "log_added".into(), payload });
                }
            }
            Parsed::Session(id) => {
                let _ = TaskService::set_session(pool, task_id, &id).await;
            }
            Parsed::Result(result) => {
                let _ = TaskService::set_result(pool, task_id, &result).await;
            }
            Parsed::Usage(usage) => {
                let _ = TaskService::add_usage(pool, task_id, &usage).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(model: Option<&str>, auto_approve: bool) -> Task {
        let mut t = Task::new("t".into(), "做點事 --model evil; rm -rf /".into(), "human".into(), None);
        t.model = model.map(Into::into);
        t.auto_approve = auto_approve;
        t
    }

    #[test]
    fn builds_agy_command_with_flags_before_prompt() {
        let argv = command_for("agy", &task(Some("gemini-4-argon-high"), false)).unwrap();
        assert_eq!(
            argv,
            [
                "agy",
                "--output-format",
                "stream-json",
                "--model",
                "gemini-4-argon-high",
                "--mode",
                "accept-edits",
                "-p",
                "做點事 --model evil; rm -rf /"
            ]
        );
    }

    #[test]
    fn builds_claude_command_and_full_access_flags() {
        let argv = command_for("claude", &task(None, true)).unwrap();
        assert_eq!(argv[..6], ["claude", "--output-format", "stream-json", "--verbose", "--permission-mode", "bypassPermissions"]);
        assert_eq!(argv.last().unwrap(), "做點事 --model evil; rm -rf /");
        let agy = command_for("agy", &task(Some(""), true)).unwrap();
        assert_eq!(agy[..4], ["agy", "--output-format", "stream-json", "--dangerously-skip-permissions"]);
    }

    #[test]
    fn resumes_existing_session() {
        let mut t = task(None, false);
        t.session_id = Some("sess-1".into());
        let claude = command_for("claude", &t).unwrap();
        assert!(claude.windows(2).any(|w| w == ["--resume", "sess-1"]), "{claude:?}");
        let agy = command_for("agy", &t).unwrap();
        assert!(agy.windows(2).any(|w| w == ["--conversation", "sess-1"]), "{agy:?}");
        assert_eq!(agy[agy.len() - 2], "-p");
    }

    #[test]
    fn unknown_agent_has_no_command() {
        assert!(command_for("rogue", &task(None, false)).is_none());
    }
}
