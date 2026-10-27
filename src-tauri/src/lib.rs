pub mod db;
pub mod features;
pub mod server;

use server::{start_background_workers, start_http_server, BoardEvent, ServerState};
use tokio::sync::broadcast;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            // 初始化 SQLite 資料庫與後端服務
            let (event_tx, _) = broadcast::channel::<BoardEvent>(100);
            let event_tx_clone = event_tx.clone();

            tauri::async_runtime::spawn(async move {
                let db_path = "agent_board.db";
                let pool = db::init_db(db_path)
                    .await
                    .expect("Failed to initialize database");

                // 啟動逾時重置背景任務
                start_background_workers(pool.clone(), event_tx_clone.clone()).await;

                // 啟動內嵌 HTTP API 服務 (預設埠號 3000)
                let server_state = ServerState {
                    pool,
                    event_tx: event_tx_clone,
                };

                let port = std::env::var("PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(3000);

                println!("[*] Agent Board API Server running on http://127.0.0.1:{}", port);
                if let Err(e) = start_http_server(port, server_state).await {
                    eprintln!("HTTP Server error: {}", e);
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
