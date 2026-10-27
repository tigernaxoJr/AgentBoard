use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RunnerStatus {
    pub agent_id: String,
    pub last_seen: DateTime<Utc>,
    /// 最近一次心跳的任務；閒置輪詢時為 None
    pub current_task: Option<String>,
    pub online: bool,
}

/// 方案 B 的 Runner 在線狀態：Runner 每次領取或心跳都會經過 API，在這裡記下最後出現時間
/// 只存在記憶體中，同一個 Agent 有多個 Runner 時合併為一筆
#[derive(Clone)]
pub struct Runners {
    seen: Arc<Mutex<HashMap<String, (DateTime<Utc>, Option<String>)>>>,
    /// 超過這段時間沒有出現就視為離線（與租約逾時相同）
    offline_after: chrono::Duration,
}

impl Runners {
    pub fn new(offline_after_secs: i64) -> Self {
        Self { seen: Arc::default(), offline_after: chrono::Duration::seconds(offline_after_secs) }
    }

    /// 記錄 Runner 出現；回傳狀態是否有變化（新上線、換任務），供呼叫端決定要不要廣播
    pub fn touch(&self, agent_id: &str, current_task: Option<String>) -> bool {
        let now = Utc::now();
        let mut seen = self.seen.lock().unwrap();
        let prev = seen.insert(agent_id.to_string(), (now, current_task.clone()));
        match prev {
            Some((last, task)) => task != current_task || now - last > self.offline_after,
            None => true,
        }
    }

    pub fn list(&self) -> Vec<RunnerStatus> {
        let now = Utc::now();
        let mut list: Vec<_> = self
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(agent_id, (last_seen, current_task))| RunnerStatus {
                agent_id: agent_id.clone(),
                last_seen: *last_seen,
                current_task: current_task.clone(),
                online: now - *last_seen <= self.offline_after,
            })
            .collect();
        list.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_presence_and_reports_changes() {
        let runners = Runners::new(60);
        assert!(runners.touch("agy", None), "first sighting is a change");
        assert!(!runners.touch("agy", None), "idle poll is not a change");
        assert!(runners.touch("agy", Some("t1".into())), "picked up a task");
        assert!(!runners.touch("agy", Some("t1".into())));
        assert!(runners.touch("agy", None), "finished the task");

        let list = runners.list();
        assert_eq!(list.len(), 1);
        assert!(list[0].online);
        assert_eq!(list[0].current_task, None);
    }

    #[test]
    fn goes_offline_after_timeout() {
        let runners = Runners::new(0);
        runners.touch("claude", Some("t1".into()));
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(!runners.list()[0].online);
        assert!(runners.touch("claude", Some("t1".into())), "coming back online is a change");
    }
}
