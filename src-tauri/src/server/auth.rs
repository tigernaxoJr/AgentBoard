use super::ServerState;
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use std::fs;
use std::path::Path;
use tower_http::cors::{AllowOrigin, CorsLayer};

/// 只允許看板自己的視窗跨來源呼叫 API（Tauri 正式版 / Windows / Vite 開發伺服器）
const ALLOWED_ORIGINS: [&str; 3] = ["tauri://localhost", "http://tauri.localhost", "http://localhost:1420"];

/// 讀取既有 token，沒有就產生一組新的並寫入檔案（Unix 上權限 0600）
/// token 跨重啟保持不變，Runner 才不必每次重新設定
pub fn load_or_create_token(path: &Path) -> std::io::Result<String> {
    if let Ok(existing) = fs::read_to_string(path) {
        let token = existing.trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(ALLOWED_ORIGINS.map(HeaderValue::from_static)))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
}

/// 所有 /api 請求都需要 `Authorization: Bearer <token>`
/// EventSource 無法自訂標頭，因此 /api/events 另外接受 `?token=<token>`
pub async fn require_token(State(state): State<ServerState>, req: Request, next: Next) -> Result<Response, StatusCode> {
    let from_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let from_query = (req.uri().path() == "/api/events")
        .then(|| req.uri().query())
        .flatten()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));

    match from_header.or(from_query) {
        Some(token) if token == state.api_token.as_str() => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::create_router;
    use axum::body::Body;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;

    const TOKEN: &str = "test-token";

    async fn router() -> axum::Router {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        let (event_tx, _) = tokio::sync::broadcast::channel(10);
        let spawner = crate::features::execution::Spawner::new(pool.clone(), event_tx.clone(), std::time::Duration::from_secs(1), 4, std::env::temp_dir());
        create_router(ServerState { pool, event_tx, api_token: Arc::new(TOKEN.into()), spawner, data_dir: std::env::temp_dir() })
    }

    async fn send(req: axum::http::request::Builder) -> Response {
        router().await.oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn rejects_requests_without_valid_token() {
        assert_eq!(send(Request::get("/api/tasks")).await.status(), StatusCode::UNAUTHORIZED);
        let wrong = Request::get("/api/tasks").header("authorization", "Bearer nope");
        assert_eq!(send(wrong).await.status(), StatusCode::UNAUTHORIZED);
        let ok = Request::get("/api/tasks").header("authorization", format!("Bearer {TOKEN}"));
        assert_eq!(send(ok).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn query_token_only_accepted_for_sse() {
        let sse = send(Request::get(format!("/api/events?token={TOKEN}"))).await;
        assert_eq!(sse.status(), StatusCode::OK);
        let tasks = send(Request::get(format!("/api/tasks?token={TOKEN}"))).await;
        assert_eq!(tasks.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn cors_only_allows_board_window() {
        let preflight = |origin: &str| {
            Request::options("/api/tasks")
                .header("origin", origin)
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "authorization,content-type")
        };
        let evil = send(preflight("https://evil.example")).await;
        assert!(evil.headers().get("access-control-allow-origin").is_none());
        let board = send(preflight("tauri://localhost")).await;
        assert_eq!(board.headers().get("access-control-allow-origin").unwrap(), "tauri://localhost");
    }

    #[test]
    fn token_file_is_created_once_and_reused() {
        let path = std::env::temp_dir().join(format!("agent_board_token_{}", uuid::Uuid::new_v4())).join("api_token");
        let first = load_or_create_token(&path).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(load_or_create_token(&path).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
