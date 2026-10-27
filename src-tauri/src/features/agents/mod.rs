use crate::features::tasks::Task;
use serde::Serialize;

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
    let mut argv = vec![agent_id.to_string()];
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task(model: Option<&str>, auto_approve: bool) -> Task {
        let mut t = Task::new("t".into(), "做點事 --model evil; rm -rf /".into(), "human".into(), None, None);
        t.model = model.map(Into::into);
        t.auto_approve = auto_approve;
        t
    }

    #[test]
    fn builds_agy_command_with_flags_before_prompt() {
        let argv = command_for("agy", &task(Some("gemini-4-argon-high"), false)).unwrap();
        assert_eq!(
            argv,
            ["agy", "--model", "gemini-4-argon-high", "--mode", "accept-edits", "-p", "做點事 --model evil; rm -rf /"]
        );
    }

    #[test]
    fn builds_claude_command_and_full_access_flags() {
        let argv = command_for("claude", &task(None, true)).unwrap();
        assert_eq!(argv[..3], ["claude", "--permission-mode", "bypassPermissions"]);
        assert_eq!(argv.last().unwrap(), "做點事 --model evil; rm -rf /");
        let agy = command_for("agy", &task(Some(""), true)).unwrap();
        assert_eq!(agy[..2], ["agy", "--dangerously-skip-permissions"]);
    }

    #[test]
    fn unknown_agent_has_no_command() {
        assert!(command_for("rogue", &task(None, false)).is_none());
    }
}
