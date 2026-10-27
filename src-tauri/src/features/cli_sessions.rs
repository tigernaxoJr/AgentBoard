//! 列出本機 claude CLI 已存在的 session（含非本看板建立的），供面板接續。
//! 只讀 `~/.claude/projects/*/<session_id>.jsonl` 的開頭，不解析整份對話。

use serde::Serialize;
use std::io::Read;
use std::path::Path;

/// 只讀檔頭：session 檔可達數十 MB，標題與 cwd 都在最前面
const HEAD_BYTES: u64 = 256 * 1024;
const MAX_SESSIONS: usize = 200;

#[derive(Debug, Serialize, PartialEq)]
pub struct CliSession {
    pub agent: &'static str,
    pub session_id: String,
    pub cwd: String,
    pub title: String,
    pub updated_at: String,
}

/// 掃描 claude 的 projects 目錄，依最後修改時間新到舊，最多 MAX_SESSIONS 筆
pub fn list_claude_sessions(projects_dir: &Path) -> Vec<CliSession> {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    let Ok(dirs) = std::fs::read_dir(projects_dir) else {
        return Vec::new();
    };
    for dir in dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                    files.push((m, p));
                }
            }
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .filter_map(|(mtime, p)| parse_head(&p, mtime))
        .take(MAX_SESSIONS)
        .collect()
}

fn parse_head(path: &Path, mtime: std::time::SystemTime) -> Option<CliSession> {
    let session_id = path.file_stem()?.to_str()?.to_string();
    let mut buf = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_string(&mut buf)
        .ok()?;
    let (mut cwd, mut title, mut custom) = (None, None, None);
    for line in buf.lines() {
        // 最後一行可能被截斷，解析失敗直接略過
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = v["cwd"]
                .as_str()
                .or_else(|| v["attachment"]["snapshot"]["workingDirectory"].as_str())
                .map(String::from);
        }
        if custom.is_none() {
            custom = v["customTitle"].as_str().map(String::from);
        }
        if title.is_none() && v["type"] == "user" {
            // 以 < 開頭的是本機指令回報等系統包裝，不是使用者的提問
            title = v["message"]["content"]
                .as_str()
                .filter(|s| !s.trim_start().starts_with('<'))
                .map(String::from);
        }
    }
    let title: String = custom
        .or(title)?
        .lines()
        .next()?
        .chars()
        .take(80)
        .collect();
    Some(CliSession {
        agent: "claude",
        session_id,
        cwd: cwd?,
        title,
        updated_at: chrono::DateTime::<chrono::Utc>::from(mtime).to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_sessions_with_title_and_cwd() {
        let root = std::env::temp_dir().join(format!("cli-sess-{}", uuid::Uuid::new_v4()));
        let proj = root.join("-p");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("s1.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/w\",\"message\":{\"content\":\"<command-name>x</command-name>\"}}\n\
             {\"type\":\"user\",\"cwd\":\"/w\",\"message\":{\"content\":\"修 bug\\n細節\"}}\n{\"type\":\"us",
        )
        .unwrap();
        // 沒有 cwd 的檔案無法接續，要略過
        std::fs::write(proj.join("s2.jsonl"), "{\"type\":\"mode\"}\n").unwrap();
        let got = list_claude_sessions(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].session_id, "s1");
        assert_eq!(got[0].cwd, "/w");
        assert_eq!(got[0].title, "修 bug");
    }
}
