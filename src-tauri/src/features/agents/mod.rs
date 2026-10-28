pub mod stream;

use crate::features::tasks::{AddTaskLogDto, Task, TaskService};
use crate::server::BoardEvent;
use serde::{Deserialize, Serialize};
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
    let mut argv = vec![
        agent_id.to_string(),
        "--output-format".into(),
        "stream-json".into(),
    ];
    if agent_id == "claude" {
        // claude 的 -p 搭配 stream-json 必須加 --verbose
        argv.push("--verbose".into());
    }
    // 有工作階段 ID（追加指示、或重試先前中斷的執行）時接續同一個對話
    if let Some(session) = task.session_id.as_deref() {
        let flag = if agent_id == "claude" {
            "--resume"
        } else {
            "--conversation"
        };
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
    // 斜線指令（如 /compact）必須是完整的 prompt，不能附加說明
    let prompt = if task.prompt.starts_with('/') {
        task.prompt.clone()
    } else {
        format!("{}\n\n{ASK_HINT}", task.prompt)
    };
    argv.extend(["-p".into(), prompt]);
    Some(argv)
}

/// 附在每個 prompt 後的提問約定：headless 模式下 Agent 的原生提問工具不可用（claude 停用、agy 不輸出內容），
/// 改由 Agent 在結果結尾附 ask 區塊，前端解析成選項按鈕（見 src/ui.ts 的 parseAsk）
pub(crate) const ASK_HINT: &str = "[Board] If you cannot continue without the user's decision, stop and end your final reply with one fenced block tagged `ask` containing JSON: {\"question\":\"...\",\"options\":[\"A\",\"B\"]}. Otherwise do not output it.";

/// 處理 Agent 輸出的一行：寫日誌、記錄工作階段與結果，並廣播日誌事件
pub async fn ingest_output(
    pool: &Pool<Sqlite>,
    event_tx: &broadcast::Sender<BoardEvent>,
    task_id: &str,
    agent: &str,
    line: &str,
) {
    for parsed in stream::parse_line(line) {
        match parsed {
            Parsed::Log(level, message) => {
                let dto = AddTaskLogDto {
                    agent_id: agent.into(),
                    level: Some(level.into()),
                    message,
                };
                if let Ok(log) = TaskService::add_log(pool, task_id, dto).await {
                    let payload = serde_json::to_value(&log).unwrap_or_default();
                    let _ = event_tx.send(BoardEvent {
                        event_type: "log_added".into(),
                        payload,
                    });
                }
            }
            Parsed::Session(id) => {
                let _ = TaskService::set_session(pool, task_id, &id).await;
            }
            Parsed::Result(result) => {
                let _ = TaskService::set_result(pool, task_id, &result).await;
                // Agent 自己回報失敗時，不論程序結束碼是什麼都算失敗，否則下游任務會拿錯誤的結果繼續跑
                if result.is_error {
                    let reason = format!("Agent 回報執行失敗：{}", result.text.trim());
                    if let Ok(true) = TaskService::fail_running_task(pool, task_id, &reason).await {
                        let _ = event_tx.send(BoardEvent {
                            event_type: "task_failed".into(),
                            payload: serde_json::json!({ "id": task_id }),
                        });
                    }
                }
            }
            Parsed::Usage(usage) => {
                let _ = TaskService::add_usage(pool, task_id, &usage).await;
            }
        }
    }
}

/// 檢查或升級 Agent CLI 的回傳結果
#[derive(Debug, Serialize)]
pub struct CliUpdateResult {
    pub agent_id: String,
    pub installed: bool,
    pub current_version: Option<String>,
    pub output: String,
    pub success: bool,
}

pub async fn check_or_update_cli(agent_id: &str) -> Result<CliUpdateResult, String> {
    if !AGENTS.iter().any(|a| a.id == agent_id) {
        return Err(format!("不支援的 Agent: {agent_id}"));
    }

    // 1. 取得目前版本
    let ver_out = tokio::process::Command::new(agent_id)
        .arg("--version")
        .output()
        .await;

    let (installed, current_version) = match ver_out {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (true, Some(ver))
        }
        _ => (false, None),
    };

    if !installed {
        return Ok(CliUpdateResult {
            agent_id: agent_id.to_string(),
            installed: false,
            current_version: None,
            output: format!("找不到 {agent_id} 執行檔，請確認已安裝至 PATH"),
            success: false,
        });
    }

    // 2. 執行 update 指令（agy update 或 claude update）
    let update_out = tokio::process::Command::new(agent_id)
        .arg("update")
        .output()
        .await
        .map_err(|e| format!("執行 {agent_id} update 失敗: {e}"))?;

    let stdout = String::from_utf8_lossy(&update_out.stdout);
    let stderr = String::from_utf8_lossy(&update_out.stderr);
    let combined = format!("{stdout}\n{stderr}").trim().to_string();

    Ok(CliUpdateResult {
        agent_id: agent_id.to_string(),
        installed: true,
        current_version,
        output: combined,
        success: update_out.status.success(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeUsageInfo {
    pub session_percent: Option<f64>,
    pub session_resets_at: Option<String>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<String>,
    pub spend_used: Option<f64>,
    pub spend_limit: Option<f64>,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgyUsageInfo {
    pub today_input_tokens: i64,
    pub today_output_tokens: i64,
    pub today_total_tokens: i64,
    pub today_active_sessions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUsageSummary {
    pub claude: Option<ClaudeUsageInfo>,
    pub agy: Option<AgyUsageInfo>,
}

/// 零 Token 消耗查詢各 Agent CLI 的本地用量與額度狀況
pub async fn get_cli_usage_stats() -> AgentUsageSummary {
    // Windows 沒有 HOME 環境變數
    let home = dirs::home_dir().unwrap_or_else(|| ".".into());

    // 1. Claude: 讀取 ~/.claude.json 中的 cachedUsageUtilization（CLI 自帶快取）
    let claude_info = {
        let claude_json_path = home.join(".claude.json");
        if let Ok(content) = std::fs::read_to_string(claude_json_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                let cached = &v["cachedUsageUtilization"];
                let util = &cached["utilization"];
                let five_h = &util["five_hour"];
                let seven_d = &util["seven_day"];
                let spend = &util["spend"];

                let spend_used = spend["used"]["amount_minor"].as_f64().map(|amt| {
                    let exp = spend["used"]["exponent"].as_f64().unwrap_or(2.0);
                    amt / 10f64.powf(exp)
                });
                let spend_limit = spend["limit"]["amount_minor"].as_f64().map(|amt| {
                    let exp = spend["limit"]["exponent"].as_f64().unwrap_or(2.0);
                    amt / 10f64.powf(exp)
                });
                let currency = spend["used"]["currency"].as_str().unwrap_or("USD").to_string();

                Some(ClaudeUsageInfo {
                    session_percent: five_h["utilization"].as_f64(),
                    session_resets_at: five_h["resets_at"].as_str().map(|s| s.to_string()),
                    weekly_percent: seven_d["utilization"].as_f64(),
                    weekly_resets_at: seven_d["resets_at"].as_str().map(|s| s.to_string()),
                    spend_used,
                    spend_limit,
                    currency,
                })
            } else {
                None
            }
        } else {
            None
        }
    };

    // 2. Agy: 彙整 ~/.gemini/antigravity-cli/brain 今日產生之 transcript 本機 Token 計量
    let agy_info = {
        let brain_dir = home.join(".gemini/antigravity-cli/brain");
        let today_prefix = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut in_tokens = 0i64;
        let mut out_tokens = 0i64;
        let mut active_sessions = 0i64;

        if let Ok(entries) = std::fs::read_dir(brain_dir) {
            for entry in entries.flatten() {
                let transcript_path = entry.path().join(".system_generated/logs/transcript.jsonl");
                if !transcript_path.is_file() {
                    continue;
                }

                if let Ok(metadata) = transcript_path.metadata() {
                    if let Ok(mtime) = metadata.modified() {
                        let mtime_date = chrono::DateTime::<chrono::Local>::from(mtime).format("%Y-%m-%d").to_string();
                        if mtime_date != today_prefix {
                            continue;
                        }
                    }
                }

                if let Ok(file) = std::fs::File::open(&transcript_path) {
                    use std::io::{BufRead, BufReader};
                    let reader = BufReader::new(file);
                    let mut session_counted = false;
                    for line in reader.lines().flatten() {
                        if line.contains("input_tokens") {
                            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                                if let Some(created) = obj["created_at"].as_str() {
                                    if created.starts_with(&today_prefix) {
                                        in_tokens += obj["input_tokens"].as_i64().unwrap_or(0);
                                        out_tokens += obj["output_tokens"].as_i64().unwrap_or(0);
                                        if !session_counted {
                                            active_sessions += 1;
                                            session_counted = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Some(AgyUsageInfo {
            today_input_tokens: in_tokens,
            today_output_tokens: out_tokens,
            today_total_tokens: in_tokens + out_tokens,
            today_active_sessions: active_sessions,
        })
    };

    AgentUsageSummary {
        claude: claude_info,
        agy: agy_info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(model: Option<&str>, auto_approve: bool) -> Task {
        let mut t = Task::new(
            "t".into(),
            "做點事 --model evil; rm -rf /".into(),
            "human".into(),
            None,
        );
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
                &format!("做點事 --model evil; rm -rf /\n\n{ASK_HINT}")
            ]
        );
    }

    #[test]
    fn builds_claude_command_and_full_access_flags() {
        let argv = command_for("claude", &task(None, true)).unwrap();
        assert_eq!(
            argv[..6],
            [
                "claude",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "bypassPermissions"
            ]
        );
        assert!(argv.last().unwrap().starts_with("做點事 --model evil; rm -rf /\n\n"));
        let agy = command_for("agy", &task(Some(""), true)).unwrap();
        assert_eq!(
            agy[..4],
            [
                "agy",
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions"
            ]
        );
    }

    #[test]
    fn resumes_existing_session() {
        let mut t = task(None, false);
        t.session_id = Some("sess-1".into());
        let claude = command_for("claude", &t).unwrap();
        assert!(
            claude.windows(2).any(|w| w == ["--resume", "sess-1"]),
            "{claude:?}"
        );
        let agy = command_for("agy", &t).unwrap();
        assert!(
            agy.windows(2).any(|w| w == ["--conversation", "sess-1"]),
            "{agy:?}"
        );
        assert_eq!(agy[agy.len() - 2], "-p");
    }

    #[test]
    fn slash_command_prompt_is_sent_as_is() {
        let mut t = task(None, false);
        t.prompt = "/compact".into();
        assert_eq!(command_for("claude", &t).unwrap().last().unwrap(), "/compact");
    }

    #[test]
    fn unknown_agent_has_no_command() {
        assert!(command_for("rogue", &task(None, false)).is_none());
    }
}
