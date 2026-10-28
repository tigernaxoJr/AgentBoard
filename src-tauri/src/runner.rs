use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

const APP_IDENTIFIER: &str = "com.naxo.agent-board";
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:18833";
pub const DEFAULT_INTERVAL_SECS: u64 = 3;
const HEARTBEAT_SECS: u64 = 15;
const FLUSH_INTERVAL_MS: u64 = 500;
const BATCH_LINES: usize = 50;
const FINISH_RETRIES: u32 = 5;
/// 伺服器不可達時最多暫存多少行輸出
const MAX_BUFFERED_LINES: usize = 2000;

/// 一批輸出的送達結果
enum Sent {
    Delivered,
    /// 租約已失效（任務被取消或回收）
    LeaseLost,
    /// 暫時失敗，輸出要保留重送
    Retry,
}

fn get_default_token_file() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_IDENTIFIER)
        .join("api_token")
}

fn resolve_token(token_arg: Option<&str>, token_file_arg: Option<&str>) -> String {
    if let Some(t) = token_arg {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(env_token) = std::env::var("AGENT_BOARD_TOKEN") {
        let trimmed = env_token.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let path = token_file_arg
        .map(PathBuf::from)
        .unwrap_or_else(get_default_token_file);
    if let Ok(file_token) = std::fs::read_to_string(path) {
        let trimmed = file_token.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    String::new()
}

#[derive(Clone)]
struct BoardClient {
    server_url: String,
    token: String,
    agent_id: String,
}

impl BoardClient {
    fn new(server_url: String, token: String, agent_id: String) -> Self {
        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            token,
            agent_id,
        }
    }

    /// ureq 是阻塞式 HTTP；直接在 async 任務裡呼叫會佔住 tokio worker，拖慢心跳
    async fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, (Option<u16>, String)> {
        tokio::task::block_in_place(|| self.post_blocking(path, body))
    }

    fn post_blocking(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, (Option<u16>, String)> {
        let url = format!("{}{}", self.server_url, path);
        let mut req = ureq::post(&url);
        if !self.token.is_empty() {
            req = req.header("Authorization", &format!("Bearer {}", self.token));
        }
        match req.send_json(body) {
            Ok(mut res) => {
                let text = res
                    .body_mut()
                    .read_to_string()
                    .unwrap_or_default();
                if text.trim().is_empty() {
                    return Ok(serde_json::json!({ "status": res.status().as_u16() }));
                }
                serde_json::from_str(&text).or(Ok(serde_json::Value::String(text)))
            }
            Err(ureq::Error::StatusCode(code)) => Err((Some(code), format!("HTTP Error {code}"))),
            Err(e) => Err((None, format!("Connection error: {e}"))),
        }
    }

    async fn claim(&self) -> Option<serde_json::Value> {
        match self.post("/api/tasks/claim", serde_json::json!({ "agent_id": self.agent_id })).await {
            Ok(val) => {
                if val.is_null() {
                    None
                } else {
                    Some(val)
                }
            }
            Err((Some(401), _)) => {
                eprintln!("[{}] API token 無效，請確認 token 檔是否為目前看板產生的。", self.agent_id);
                std::process::exit(1);
            }
            Err((_, _e)) => {
                // Connection fail or temporary network issue
                None
            }
        }
    }

    async fn heartbeat(&self, task_id: &str, lease_id: &str) -> bool {
        match self.post(&format!("/api/tasks/{task_id}/heartbeat"), serde_json::json!({ "lease_id": lease_id })).await {
            Ok(_) => true,
            Err((Some(409), _)) => false, // 409 Conflict = 租約已失效
            Err(_) => true,               // 暫時網路錯誤視為租約尚存
        }
    }

    async fn log(&self, task_id: &str, message: &str, level: &str) {
        let _ = self
            .post(
                &format!("/api/tasks/{task_id}/logs"),
                serde_json::json!({
                    "agent_id": self.agent_id,
                    "level": level,
                    "message": message
                }),
            )
            .await;
    }

    async fn output(&self, task_id: &str, lease_id: &str, lines: &[String]) -> Sent {
        match self
            .post(
                &format!("/api/tasks/{task_id}/output"),
                serde_json::json!({
                    "agent_id": self.agent_id,
                    "lease_id": lease_id,
                    "lines": lines
                }),
            )
            .await
        {
            Ok(_) => Sent::Delivered,
            Err((Some(409), _)) => Sent::LeaseLost,
            // 連線失敗或伺服器暫時出錯：輸出要留著重送，最終結果行（result 事件）掉了任務就沒有結果文字
            Err(_) => Sent::Retry,
        }
    }

    /// 送出並清空一批輸出；暫時失敗時保留，下次一併重送（最多保留 MAX_BUFFERED_LINES 行，超過丟掉最舊的）
    async fn flush(&self, task_id: &str, lease_id: &str, batch: &mut Vec<String>, lease_lost: &AtomicBool) {
        if batch.is_empty() {
            return;
        }
        if lease_lost.load(Ordering::SeqCst) {
            batch.clear();
            return;
        }
        match self.output(task_id, lease_id, batch).await {
            Sent::Delivered => batch.clear(),
            Sent::LeaseLost => {
                lease_lost.store(true, Ordering::SeqCst);
                batch.clear();
            }
            Sent::Retry => {
                if batch.len() > MAX_BUFFERED_LINES {
                    batch.drain(..batch.len() - MAX_BUFFERED_LINES);
                }
            }
        }
    }

    /// 回報結果；連線失敗或伺服器錯誤時重試，否則租約逾時後任務會被放回佇列重跑
    /// 4xx（含租約失效的 409）重試也不會成功，直接放棄
    /// outcome：Ok = 完成，Err(原因) = 失敗
    async fn finish(&self, task_id: &str, lease_id: &str, outcome: Result<(), &str>) {
        let (action, body) = match outcome {
            Ok(()) => ("complete", serde_json::json!({ "lease_id": lease_id })),
            Err(reason) => (
                "fail",
                serde_json::json!({ "lease_id": lease_id, "reason": reason }),
            ),
        };
        for attempt in 0..FINISH_RETRIES {
            let res = self
                .post(&format!("/api/tasks/{task_id}/{action}"), body.clone())
                .await;
            match res {
                Ok(_) => return,
                Err((Some(code), _)) if code < 500 => return,
                Err((_, e)) => {
                    eprintln!("[{}] 回報 {action} 失敗（第 {} 次）：{e}", self.agent_id, attempt + 1);
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
            }
        }
        eprintln!("[{}] 無法回報任務 {task_id} 的結果，等待租約逾時後由看板回收", self.agent_id);
    }
}

pub struct RunnerConfig {
    pub agent: String,
    pub server: String,
    pub interval_secs: u64,
    pub token: Option<String>,
    pub token_file: Option<String>,
    pub dry_run: bool,
    pub dry_run_seconds: u64,
    pub once: bool,
}

pub async fn run_runner(config: RunnerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let token = resolve_token(config.token.as_deref(), config.token_file.as_deref());
    let client = BoardClient::new(config.server.clone(), token, config.agent.clone());

    println!("[*] Agent Board Runner (Native) 啟動");
    println!("[*] Agent ID: {}", config.agent);
    println!("[*] Server: {}", config.server);
    println!("[*] Polling 間隔: {} 秒", config.interval_secs);
    println!("[*] 等待領取任務中...\n");

    loop {
        if let Some(task) = client.claim().await {
            execute_task(&client, &task, &config).await;
            if config.once {
                return Ok(());
            }
            continue;
        }

        if config.once {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(config.interval_secs)).await;
    }
}

async fn execute_task(client: &BoardClient, task: &serde_json::Value, config: &RunnerConfig) {
    let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or_default();
    let lease_id = task.get("lease_id").and_then(|v| v.as_str()).unwrap_or_default();
    let title = task.get("title").and_then(|v| v.as_str()).unwrap_or_default();
    let prompt = task.get("prompt").and_then(|v| v.as_str()).unwrap_or_default();
    let attempts = task.get("attempts").and_then(|v| v.as_i64()).unwrap_or(1);

    println!("\n==========================================");
    println!("🚀 領取任務 #{}: {} (第 {} 次)", &task_id[..task_id.len().min(8)], title, attempts);
    println!("Prompt: {prompt}");
    println!("==========================================\n");

    client.log(task_id, &format!("開始執行任務: {title}"), "INFO").await;

    if config.dry_run {
        println!("[Dry-run] 模擬任務執行中（{} 秒）...", config.dry_run_seconds);
        tokio::time::sleep(Duration::from_secs(config.dry_run_seconds)).await;
        client.finish(task_id, lease_id, Ok(())).await;
        client.log(task_id, "任務模擬執行完成！", "INFO").await;
        return;
    }

    let command_arr = task.get("command").and_then(|v| v.as_array());
    let Some(raw_cmd) = command_arr else {
        let reason = format!("不支援的 Agent：{}", config.agent);
        client.log(task_id, &reason, "ERROR").await;
        client.finish(task_id, lease_id, Err(&reason)).await;
        return;
    };

    let args: Vec<String> = raw_cmd
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if args.is_empty() {
        client.log(task_id, "空指令無法執行", "ERROR").await;
        client.finish(task_id, lease_id, Err("空指令無法執行")).await;
        return;
    }

    let run_dir = task
        .get("run_dir")
        .and_then(|v| v.as_str())
        .or_else(|| task.get("workdir").and_then(|v| v.as_str()));

    let cmd_display = if args.len() > 1 {
        args[..args.len() - 1].join(" ")
    } else {
        args[0].clone()
    };
    client.log(
        task_id,
        &format!("執行：{cmd_display}（工作目錄：{}）", run_dir.unwrap_or(".")),
        "INFO",
    )
    .await;

    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }
    if let Some(dir) = run_dir {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    // 獨立的 process group，中止時連同 Agent 開的工具子程序一起結束
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("無法啟動 {}：{e}", args[0]);
            eprintln!("[{}] {msg}", config.agent);
            client.log(task_id, &msg, "ERROR").await;
            client.finish(task_id, lease_id, Err(&msg)).await;
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let (line_tx, mut line_rx) = mpsc::channel::<String>(500);

    // Stdout reader
    let line_tx_stdout = line_tx.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() {
                let _ = line_tx_stdout.send(trimmed).await;
            }
        }
    });

    // Stderr reader
    let line_tx_stderr = line_tx;
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() {
                let _ = line_tx_stderr.send(trimmed).await;
            }
        }
    });

    let lease_lost = Arc::new(AtomicBool::new(false));
    let lease_lost_hb = lease_lost.clone();
    let client_hb = client.clone();
    let t_id = task_id.to_string();
    let l_id = lease_id.to_string();

    // Heartbeat task
    let hb_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
        loop {
            ticker.tick().await;
            if !client_hb.heartbeat(&t_id, &l_id).await {
                lease_lost_hb.store(true, Ordering::SeqCst);
                break;
            }
        }
    });

    // Output batcher task
    let client_out = client.clone();
    let t_id_out = task_id.to_string();
    let l_id_out = lease_id.to_string();
    let lease_lost_out = lease_lost.clone();
    let agent_id_display = config.agent.clone();

    let output_handle = tokio::spawn(async move {
        let mut batch = Vec::with_capacity(BATCH_LINES);
        let mut flush_ticker = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
        let mut rx_open = true;

        // 通道關閉（程序結束、兩條管線都讀完）後才會收到 None；
        // 計時器永遠有效，所以不能靠 select! 的 else 分支偵測，要自己記錄 rx_open
        while rx_open {
            tokio::select! {
                line = line_rx.recv() => match line {
                    Some(line) => {
                        println!("[{}] {}", agent_id_display, &line[..line.len().min(200)]);
                        batch.push(line);
                        if batch.len() >= BATCH_LINES {
                            client_out.flush(&t_id_out, &l_id_out, &mut batch, &lease_lost_out).await;
                        }
                    }
                    None => rx_open = false,
                },
                _ = flush_ticker.tick() => {
                    client_out.flush(&t_id_out, &l_id_out, &mut batch, &lease_lost_out).await;
                }
            }
        }
        // 程序已結束：把剩下的（含之前沒送出去的）盡量送完，再由外層回報完成 / 失敗
        for attempt in 0..FINISH_RETRIES {
            client_out.flush(&t_id_out, &l_id_out, &mut batch, &lease_lost_out).await;
            if batch.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
        }
    });

    // Wait for child process exit or heartbeat failure
    let exit_status = loop {
        tokio::select! {
            res = child.wait() => {
                break Some(res);
            }
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if lease_lost.load(Ordering::SeqCst) {
                    crate::features::execution::kill_tree(&mut child).await;
                    break None;
                }
            }
        }
    };

    hb_handle.abort();
    let _ = output_handle.await;

    if lease_lost.load(Ordering::SeqCst) || exit_status.is_none() {
        println!("[{}] 租約已失效，已中止（任務被取消或回收）", config.agent);
        return;
    }

    match exit_status.unwrap() {
        Ok(status) if status.success() => {
            client.finish(task_id, lease_id, Ok(())).await;
            client.log(task_id, "任務已成功完成！", "INFO").await;
        }
        Ok(status) => {
            let code = status.code().unwrap_or(-1);
            let reason = format!("程序異常結束，結束碼：{code}");
            client.finish(task_id, lease_id, Err(&reason)).await;
            client.log(task_id, &reason, "ERROR").await;
        }
        Err(e) => {
            let reason = format!("等待程序時發生錯誤：{e}");
            client.finish(task_id, lease_id, Err(&reason)).await;
            client.log(task_id, &reason, "ERROR").await;
        }
    }
}
