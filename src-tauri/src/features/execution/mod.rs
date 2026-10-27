use crate::features::agents::AGENTS;
use crate::features::tasks::{AddTaskLogDto, ClaimedTask, RunOptions, Task, TaskService};
use crate::server::BoardEvent;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug)]
pub enum SpawnError {
    UnsupportedAgent,
    /// 任務不在佇列中，或前置任務尚未完成
    NotClaimable,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for SpawnError {
    fn from(e: sqlx::Error) -> Self {
        SpawnError::Db(e)
    }
}

enum Outcome {
    Exited(std::io::Result<std::process::ExitStatus>),
    /// 心跳被拒或收到取消：任務已被取消或回收，狀態由對方決定
    Aborted,
}

/// 方案 A：由面板後端直接拉起 Agent 子程序
/// 與 Runner 一樣透過租約領取並送心跳，因此兩者不會執行到同一個任務
#[derive(Clone)]
pub struct Spawner {
    pool: Pool<Sqlite>,
    event_tx: broadcast::Sender<BoardEvent>,
    heartbeat_every: Duration,
    running: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl Spawner {
    pub fn new(pool: Pool<Sqlite>, event_tx: broadcast::Sender<BoardEvent>, heartbeat_every: Duration) -> Self {
        Self { pool, event_tx, heartbeat_every, running: Arc::default() }
    }

    /// 寫入執行設定、領取指定任務並啟動子程序
    pub async fn spawn(&self, task_id: &str, opts: RunOptions) -> Result<Task, SpawnError> {
        let agent_id = opts.agent_id.clone().ok_or(SpawnError::UnsupportedAgent)?;
        if !AGENTS.iter().any(|a| a.id == agent_id) {
            return Err(SpawnError::UnsupportedAgent);
        }
        let claimed = TaskService::claim_specific(&self.pool, task_id, &agent_id, &opts)
            .await?
            .ok_or(SpawnError::NotClaimable)?;
        let argv = claimed.command.clone().ok_or(SpawnError::UnsupportedAgent)?;

        self.emit("task_claimed", serde_json::to_value(&claimed.task).unwrap_or_default());
        let task = claimed.task.clone();
        self.start(claimed, argv);
        Ok(task)
    }

    /// 中止本機正在執行的任務；任務不是由這裡啟動的則回傳 false
    pub fn kill(&self, task_id: &str) -> bool {
        match self.running.lock().unwrap().remove(task_id) {
            Some(tx) => tx.send(()).is_ok(),
            None => false,
        }
    }

    /// 啟動子程序並在背景監看（測試可直接傳入任意 argv）
    pub fn start(&self, claimed: ClaimedTask, argv: Vec<String>) -> JoinHandle<()> {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let task_id = claimed.task.id.clone();
        self.running.lock().unwrap().insert(task_id.clone(), cancel_tx);

        let this = self.clone();
        tokio::spawn(async move {
            this.supervise(claimed, argv, cancel_rx).await;
            this.running.lock().unwrap().remove(&task_id);
        })
    }

    async fn supervise(&self, claimed: ClaimedTask, argv: Vec<String>, mut cancel_rx: oneshot::Receiver<()>) {
        let task_id = claimed.task.id.as_str();
        let lease_id = claimed.lease_id.as_str();
        let agent = claimed.task.assigned_agent.clone().unwrap_or_default();
        let workdir = claimed
            .task
            .workdir
            .clone()
            .or_else(|| std::env::var("AGENT_BOARD_WORKDIR").ok())
            .or_else(|| std::env::var("HOME").ok());

        // prompt 是最後一個參數，日誌只記錄指令本身
        let shown = argv[..argv.len().saturating_sub(1)].join(" ");
        self.log(task_id, &agent, "INFO", format!("執行：{shown}（工作目錄：{}）", workdir.as_deref().unwrap_or(".")))
            .await;

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = &workdir {
            cmd.current_dir(dir);
        }
        // 獨立的 process group，中止時連同 Agent 開的工具子程序一起結束
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.log(task_id, &agent, "ERROR", format!("無法啟動 {}：{e}", argv[0])).await;
                self.finish(task_id, lease_id, false).await;
                return;
            }
        };

        let readers = [
            child.stdout.take().map(|out| tokio::spawn(self.clone().pipe_lines(task_id.into(), agent.clone(), out))),
            child.stderr.take().map(|err| tokio::spawn(self.clone().pipe_lines(task_id.into(), agent.clone(), err))),
        ];

        let mut heartbeat = tokio::time::interval(self.heartbeat_every);
        heartbeat.tick().await; // 第一次 tick 立即觸發，領取時已更新過心跳
        let outcome = loop {
            tokio::select! {
                status = child.wait() => break Outcome::Exited(status),
                _ = heartbeat.tick() => {
                    // 資料庫暫時出錯不視為租約失效
                    if let Ok(false) = TaskService::heartbeat(&self.pool, task_id, lease_id).await {
                        kill_tree(&mut child).await;
                        break Outcome::Aborted;
                    }
                }
                _ = &mut cancel_rx => {
                    kill_tree(&mut child).await;
                    break Outcome::Aborted;
                }
            }
        };

        // 等輸出讀完，日誌才會在狀態變更之前寫完；殘留的孫程序可能一直佔著管線，最多等 2 秒
        for reader in readers.into_iter().flatten() {
            let abort = reader.abort_handle();
            if tokio::time::timeout(Duration::from_secs(2), reader).await.is_err() {
                abort.abort();
            }
        }

        match outcome {
            Outcome::Exited(Ok(status)) if status.success() => {
                self.log(task_id, &agent, "INFO", "任務已成功完成".into()).await;
                self.finish(task_id, lease_id, true).await;
            }
            Outcome::Exited(Ok(status)) => {
                let code = status.code().map_or("無（被訊號終止）".to_string(), |c| c.to_string());
                self.log(task_id, &agent, "ERROR", format!("程序異常結束，結束碼：{code}")).await;
                self.finish(task_id, lease_id, false).await;
            }
            Outcome::Exited(Err(e)) => {
                self.log(task_id, &agent, "ERROR", format!("等待程序時發生錯誤：{e}")).await;
                self.finish(task_id, lease_id, false).await;
            }
            Outcome::Aborted => {
                self.log(task_id, &agent, "WARN", "已中止：任務被取消或租約已被回收".into()).await;
            }
        }
    }

    async fn pipe_lines(self, task_id: String, agent: String, stream: impl AsyncRead + Unpin) {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim_end();
            if !line.is_empty() {
                self.log(&task_id, &agent, "INFO", line.to_string()).await;
            }
        }
    }

    async fn finish(&self, task_id: &str, lease_id: &str, ok: bool) {
        let result = if ok {
            TaskService::complete_task(&self.pool, task_id, lease_id).await
        } else {
            TaskService::fail_task(&self.pool, task_id, lease_id).await
        };
        if let Ok(true) = result {
            let event = if ok { "task_completed" } else { "task_failed" };
            self.emit(event, serde_json::json!({ "id": task_id }));
        }
    }

    async fn log(&self, task_id: &str, agent: &str, level: &str, message: String) {
        let dto = AddTaskLogDto { agent_id: agent.into(), level: Some(level.into()), message };
        if let Ok(log) = TaskService::add_log(&self.pool, task_id, dto).await {
            self.emit("log_added", serde_json::to_value(&log).unwrap_or_default());
        }
    }

    fn emit(&self, event_type: &str, payload: serde_json::Value) {
        let _ = self.event_tx.send(BoardEvent { event_type: event_type.into(), payload });
    }
}

async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // 負的 pid 代表整個 process group
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::tasks::CreateTaskDto;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup(heartbeat_ms: u64) -> (Spawner, Pool<Sqlite>) {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        let (event_tx, _) = broadcast::channel(100);
        (Spawner::new(pool.clone(), event_tx, Duration::from_millis(heartbeat_ms)), pool)
    }

    async fn claim(pool: &Pool<Sqlite>, workdir: Option<&str>) -> ClaimedTask {
        let dto = CreateTaskDto { title: "t".into(), prompt: "p".into(), ..Default::default() };
        let task = TaskService::create_task(pool, dto).await.unwrap();
        let opts = RunOptions { workdir: workdir.map(Into::into), ..Default::default() };
        TaskService::claim_specific(pool, &task.id, "agy", &opts).await.unwrap().unwrap()
    }

    fn sh(script: &str) -> Vec<String> {
        // 最後一個參數模擬 prompt
        vec!["sh".into(), "-c".into(), script.into(), "prompt".into()]
    }

    async fn status_and_logs(pool: &Pool<Sqlite>, id: &str) -> (String, Vec<String>) {
        let status = TaskService::get_task(pool, id).await.unwrap().unwrap().status;
        let logs = TaskService::get_logs(pool, id).await.unwrap().into_iter().map(|l| l.message).collect();
        (status, logs)
    }

    async fn within(handle: JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(5), handle).await.expect("spawner did not finish").unwrap();
    }

    #[tokio::test]
    async fn successful_process_completes_task_with_logs() {
        let (spawner, pool) = setup(1000).await;
        let claimed = claim(&pool, Some("/tmp")).await;
        let id = claimed.task.id.clone();
        within(spawner.start(claimed, sh("echo hello; echo warn-line >&2; pwd"))).await;

        let (status, logs) = status_and_logs(&pool, &id).await;
        assert_eq!(status, "completed");
        for expected in ["hello", "warn-line", "/tmp"] {
            assert!(logs.iter().any(|l| l == expected), "missing log {expected}: {logs:?}");
        }
    }

    #[tokio::test]
    async fn non_zero_exit_fails_task() {
        let (spawner, pool) = setup(1000).await;
        let claimed = claim(&pool, None).await;
        let id = claimed.task.id.clone();
        within(spawner.start(claimed, sh("exit 3"))).await;

        let (status, logs) = status_and_logs(&pool, &id).await;
        assert_eq!(status, "failed");
        assert!(logs.iter().any(|l| l.contains("結束碼：3")));
    }

    #[tokio::test]
    async fn missing_binary_fails_task() {
        let (spawner, pool) = setup(1000).await;
        let claimed = claim(&pool, None).await;
        let id = claimed.task.id.clone();
        within(spawner.start(claimed, vec!["definitely-not-a-binary".into(), "prompt".into()])).await;

        let (status, logs) = status_and_logs(&pool, &id).await;
        assert_eq!(status, "failed");
        assert!(logs.iter().any(|l| l.contains("無法啟動 definitely-not-a-binary")));
    }

    #[tokio::test]
    async fn kill_stops_process_and_keeps_cancelled_status() {
        let (spawner, pool) = setup(1000).await;
        let claimed = claim(&pool, None).await;
        let id = claimed.task.id.clone();
        let handle = spawner.start(claimed, sh("sleep 30"));

        assert!(TaskService::cancel_task(&pool, &id).await.unwrap());
        assert!(spawner.kill(&id));
        within(handle).await;

        let (status, logs) = status_and_logs(&pool, &id).await;
        assert_eq!(status, "failed");
        assert!(logs.iter().any(|l| l.contains("已中止")));
        assert!(!spawner.kill(&id));
    }

    #[tokio::test]
    async fn revoked_lease_is_noticed_on_next_heartbeat() {
        let (spawner, pool) = setup(100).await;
        let claimed = claim(&pool, None).await;
        let id = claimed.task.id.clone();
        let handle = spawner.start(claimed, sh("sleep 30"));

        // 只作廢租約、不呼叫 kill：Spawner 要靠心跳被拒自行中止
        TaskService::cancel_task(&pool, &id).await.unwrap();
        within(handle).await;
        assert!(status_and_logs(&pool, &id).await.1.iter().any(|l| l.contains("已中止")));
    }

    #[tokio::test]
    async fn spawn_rejects_unknown_agent_and_unclaimable_task() {
        let (spawner, pool) = setup(1000).await;
        let task = TaskService::create_task(&pool, CreateTaskDto { title: "t".into(), prompt: "p".into(), ..Default::default() })
            .await
            .unwrap();
        let rogue = RunOptions { agent_id: Some("rogue".into()), ..Default::default() };
        assert!(matches!(spawner.spawn(&task.id, rogue).await, Err(SpawnError::UnsupportedAgent)));
        assert_eq!(TaskService::get_task(&pool, &task.id).await.unwrap().unwrap().status, "queued");

        let missing = RunOptions { agent_id: Some("agy".into()), ..Default::default() };
        assert!(matches!(spawner.spawn("no-such-task", missing).await, Err(SpawnError::NotClaimable)));
    }
}
