//! 不開視窗、只跑 API 伺服器，方便測試 Runner：
//! cargo run --example headless -- <db 路徑> <token 檔路徑>
//! 環境變數 PORT / LEASE_TIMEOUT_SECS 與 App 相同
use agent_board_lib::{serve, server::auth::load_or_create_token};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let db_path = args.next().unwrap_or_else(|| "agent_board.db".into());
    let token_path = args.next().unwrap_or_else(|| "api_token".into());

    let token =
        load_or_create_token(std::path::Path::new(&token_path)).expect("Failed to load token");
    println!("[*] API token file: {}", token_path);

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(18833);
    let lease_timeout = std::env::var("LEASE_TIMEOUT_SECS")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(60);
    serve(&db_path, token, port, lease_timeout).await;
}
