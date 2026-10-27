pub mod db;
pub mod features;
pub mod server;

use serde::Serialize;
use server::{start_background_workers, start_http_server, BoardEvent, ServerState};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::broadcast;

#[derive(Clone, Serialize)]
struct ApiConfig {
    base_url: String,
    token: String,
}

/// 前端透過 IPC 取得 API 位址與 token（token 不經過網頁可讀取的地方）
#[tauri::command]
fn get_api_config(config: tauri::State<ApiConfig>) -> ApiConfig {
    config.inner().clone()
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// 初始化資料庫、背景逾時檢查與 HTTP API；Tauri App 與 headless 範例共用
pub async fn serve(db_path: &str, api_token: String, port: u16, lease_timeout_secs: i64) {
    let (event_tx, _) = broadcast::channel::<BoardEvent>(100);
    let pool = db::init_db(db_path)
        .await
        .expect("Failed to initialize database");

    // 啟動逾時重置背景任務
    start_background_workers(pool.clone(), event_tx.clone(), lease_timeout_secs).await;

    // 方案 A 的子程序心跳頻率與 Runner 一致：逾時時間的 1/3
    let heartbeat_every = std::time::Duration::from_secs((lease_timeout_secs / 3).max(1) as u64);
    let spawner = features::execution::Spawner::new(pool.clone(), event_tx.clone(), heartbeat_every);
    let server_state = ServerState {
        pool,
        event_tx,
        api_token: Arc::new(api_token),
        spawner,
    };

    println!("[*] Agent Board API Server running on http://127.0.0.1:{}", port);
    if let Err(e) = start_http_server(port, server_state).await {
        eprintln!("HTTP Server error: {}", e);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_api_config])
        .setup(|app| {
            // token 存在 App 資料夾（Linux: ~/.local/share/com.naxo.agent-board/api_token），Runner 從這裡讀
            let token_path = app.path().app_data_dir()?.join("api_token");
            let token = server::auth::load_or_create_token(&token_path)?;
            println!("[*] API token file: {}", token_path.display());

            let port = env_or("PORT", 18833u16);
            let lease_timeout = env_or("LEASE_TIMEOUT_SECS", 60i64);
            app.manage(ApiConfig {
                base_url: format!("http://127.0.0.1:{}", port),
                token: token.clone(),
            });

            // 初始化 SQLite 資料庫與後端服務（預設埠號 18833）
            tauri::async_runtime::spawn(async move {
                serve("agent_board.db", token, port, lease_timeout).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
