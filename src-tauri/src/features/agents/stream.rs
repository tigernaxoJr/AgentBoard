use serde_json::Value;

/// Agent 一次執行的最終結果（取自 stream-json 的 result 事件）；None = 事件沒提供，沿用逐步累加的值
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunResult {
    pub text: String,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    pub is_error: bool,
}

/// 單一步驟的用量，逐步加總成這次執行的總量
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub duration_ms: i64,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Log(&'static str, String),
    Session(String),
    Result(RunResult),
    Usage(Usage),
}

/// 解析 Agent 以 `--output-format stream-json` 輸出的一行
/// claude 的事件用 `type` 區分、agy 用 `event`；不是 JSON 的行（例如 stderr）原樣當成日誌
/// 逐行解析、不保留狀態，Spawner 與 Runner 回傳的輸出都走這裡
pub fn parse_line(line: &str) -> Vec<Parsed> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![Parsed::Log("INFO", line.to_string())];
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);

    match (s(&v, "type").as_deref(), s(&v, "event").as_deref()) {
        // ── claude ──
        (Some("system"), _) if s(&v, "subtype").as_deref() == Some("init") => {
            s(&v, "session_id").map(Parsed::Session).into_iter().collect()
        }
        (Some("assistant"), _) => content(&v)
            .filter_map(|c| match s(c, "type").as_deref() {
                Some("text") => s(c, "text").filter(|t| !t.trim().is_empty()).map(|t| Parsed::Log("INFO", t)),
                Some("tool_use") => Some(tool_log(&s(c, "name").unwrap_or_default(), c.get("input"))),
                _ => None,
            })
            .collect(),
        // 工具結果很吵，只記錄失敗的
        (Some("user"), _) => content(&v)
            .filter(|c| c.get("is_error").and_then(Value::as_bool) == Some(true))
            .map(|c| Parsed::Log("WARN", truncate(&text_of(c.get("content")), 300)))
            .collect(),
        (Some("result"), _) => {
            let usage = v.get("usage").cloned().unwrap_or_default();
            let result = RunResult {
                text: s(&v, "result").unwrap_or_default(),
                cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                input_tokens: Some(
                    n(&usage, "input_tokens") + n(&usage, "cache_creation_input_tokens") + n(&usage, "cache_read_input_tokens"),
                ),
                output_tokens: Some(n(&usage, "output_tokens")),
                duration_ms: Some(n(&v, "duration_ms")),
                is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            };
            s(&v, "session_id").map(Parsed::Session).into_iter().chain([Parsed::Result(result)]).collect()
        }
        // ── agy ──
        (_, Some("init")) => s(&v, "conversation_id").map(Parsed::Session).into_iter().collect(),
        (_, Some("step_update")) => {
            let step = v.get("step_update").cloned().unwrap_or_default();
            if s(&step, "state").as_deref() != Some("DONE") {
                return vec![];
            }
            // result 事件的用量是整段對話的累計（含先前接續的回合），改由完成的步驟逐一加總
            let usage = step.get("usage").cloned().unwrap_or_default();
            let mut out = vec![Parsed::Usage(Usage {
                input_tokens: n(&usage, "input_tokens") + n(&usage, "cache_read_tokens"),
                output_tokens: n(&usage, "output_tokens") + n(&usage, "thinking_tokens"),
                duration_ms: (step.get("duration_seconds").and_then(Value::as_f64).unwrap_or(0.0) * 1000.0) as i64,
            })];
            // 回覆文字是逐字片段，最後在 result 事件一次取得；這裡只記錄完成的工具呼叫
            if s(&step, "step_type").as_deref() == Some("tool") {
                let info = step.get("tool_info").cloned().unwrap_or_default();
                out.push(tool_log(&s(&step, "tool_name").unwrap_or_default(), info.get("parameters")));
            }
            out.retain(|p| *p != Parsed::Usage(Usage::default()));
            out
        }
        (_, Some("result")) => {
            let r = v.get("result").cloned().unwrap_or_default();
            let text = s(&r, "response").unwrap_or_default().trim_end().to_string();
            let result = RunResult {
                text: text.clone(),
                is_error: s(&r, "status").as_deref() != Some("SUCCESS"),
                ..Default::default()
            };
            let mut out: Vec<_> = s(&r, "conversation_id").map(Parsed::Session).into_iter().collect();
            if !text.is_empty() {
                out.push(Parsed::Log("INFO", text));
            }
            out.push(Parsed::Result(result));
            out
        }
        // 認得是 JSON 但不是上面的事件（hook、rate limit、思考 token 統計等），略過
        (Some(_), _) | (_, Some(_)) => vec![],
        _ => vec![Parsed::Log("INFO", line.to_string())],
    }
}

fn content(v: &Value) -> impl Iterator<Item = &Value> {
    v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten()
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items.iter().filter_map(|i| i.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// 工具呼叫顯示成一行：「▸ Bash  ls -la」，優先挑最有代表性的參數
fn tool_log(name: &str, input: Option<&Value>) -> Parsed {
    const KEYS: &[&str] = &["command", "CommandLine", "file_path", "AbsolutePath", "TargetFile", "path", "pattern", "url", "query", "description"];
    let detail = input
        .and_then(|i| KEYS.iter().find_map(|k| i.get(*k).and_then(Value::as_str).map(str::to_string)))
        .or_else(|| input.filter(|i| i.as_object().is_some_and(|o| !o.is_empty())).map(Value::to_string))
        .unwrap_or_default();
    Parsed::Log("TOOL", format!("▸ {name}  {}", truncate(detail.lines().next().unwrap_or(""), 200)).trim_end().to_string())
}

pub fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(parse_line("Error: boom"), vec![Parsed::Log("INFO", "Error: boom".into())]);
        assert!(parse_line("   ").is_empty());
    }

    #[test]
    fn parses_claude_stream() {
        let init = r#"{"type":"system","subtype":"init","session_id":"s-1","tools":[]}"#;
        assert_eq!(parse_line(init), vec![Parsed::Session("s-1".into())]);
        assert!(parse_line(r#"{"type":"system","subtype":"hook_started"}"#).is_empty());

        let msg = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"好的"},{"type":"tool_use","name":"Bash","input":{"command":"ls -la\nrm x","description":"list"}}]}}"#;
        assert_eq!(
            parse_line(msg),
            vec![Parsed::Log("INFO", "好的".into()), Parsed::Log("TOOL", "▸ Bash  ls -la".into())]
        );

        let err = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"},{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"denied"}]}]}}"#;
        assert_eq!(parse_line(err), vec![Parsed::Log("WARN", "denied".into())]);

        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"OK","session_id":"s-1","total_cost_usd":0.0222,"duration_ms":2892,"usage":{"input_tokens":10,"cache_creation_input_tokens":9907,"cache_read_input_tokens":20600,"output_tokens":70}}"#;
        assert_eq!(
            parse_line(result),
            vec![
                Parsed::Session("s-1".into()),
                Parsed::Result(RunResult {
                    text: "OK".into(),
                    cost_usd: Some(0.0222),
                    input_tokens: Some(30517),
                    output_tokens: Some(70),
                    duration_ms: Some(2892),
                    is_error: false
                })
            ]
        );
    }

    #[test]
    fn parses_agy_stream() {
        let init = r#"{"event":"init","conversation_id":"c-1","init":{"model":"m"}}"#;
        assert_eq!(parse_line(init), vec![Parsed::Session("c-1".into())]);
        let delta = r#"{"event":"step_update","step_update":{"state":"ACTIVE","step_type":"agent_response","text_delta":"OK"}}"#;
        assert!(parse_line(delta).is_empty());
        let active_tool = r#"{"event":"step_update","step_update":{"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"parameters":{"CommandLine":"ls"}}}}"#;
        assert!(parse_line(active_tool).is_empty());
        let tool = r#"{"event":"step_update","step_update":{"state":"DONE","step_type":"tool","tool_name":"run_command","duration_seconds":0.33,"tool_info":{"parameters":{"CommandLine":"ls"},"output":"x"}}}"#;
        assert_eq!(
            parse_line(tool),
            vec![
                Parsed::Usage(Usage { duration_ms: 330, ..Default::default() }),
                Parsed::Log("TOOL", "▸ run_command  ls".into())
            ]
        );
        let reply = r#"{"event":"step_update","step_update":{"state":"DONE","step_type":"agent_response","duration_seconds":2.8,"usage":{"input_tokens":8969,"output_tokens":43,"thinking_tokens":1,"cache_read_tokens":0}}}"#;
        assert_eq!(parse_line(reply), vec![Parsed::Usage(Usage { input_tokens: 8969, output_tokens: 44, duration_ms: 2800 })]);
        let user = r#"{"event":"step_update","step_update":{"state":"DONE","step_type":"user_input"}}"#;
        assert!(parse_line(user).is_empty());

        let result = r#"{"event":"result","result":{"conversation_id":"c-1","status":"SUCCESS","response":"OK\n","duration_seconds":2.9494,"usage":{"input_tokens":8952,"output_tokens":1,"thinking_tokens":2,"cache_read_tokens":5}}}"#;
        assert_eq!(
            parse_line(result),
            vec![
                Parsed::Session("c-1".into()),
                Parsed::Log("INFO", "OK".into()),
                Parsed::Result(RunResult { text: "OK".into(), ..Default::default() })
            ]
        );
    }

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate("中文字串", 2), "中文…");
        assert_eq!(truncate("ab", 5), "ab");
    }
}
