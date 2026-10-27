use crate::features::agents::{self, AGENTS};
use crate::features::artifacts::{ArtifactService, CreateArtifactDto};
use crate::features::execution::{SpawnError, Spawner};
use crate::features::runners::Runners;
use crate::features::worktree;
use crate::features::messages::{MessageService, SendMessageDto};
use crate::features::tasks::{AddTaskLogDto, ClaimTaskDto, CreateTaskDto, LeaseDto, RequeueResult, RunOptions, TaskService};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::net::SocketAddr;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct BoardEvent {
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub struct ServerState {
    pub pool: Pool<Sqlite>,
    pub event_tx: broadcast::Sender<BoardEvent>,
    pub api_token: Arc<String>,
    pub spawner: Spawner,
    pub runners: Runners,
    /// 資料庫與 worktree 所在的資料夾
    pub data_dir: PathBuf,
}

#[derive(Deserialize)]
pub struct GetMessagesQuery {
    pub receiver: String,
    pub unread: Option<bool>,
}

pub fn create_router(state: ServerState) -> Router {
    Router::new()
        // Tasks
        .route("/api/tasks", post(create_task).get(list_tasks))
        .route("/api/tasks/{id}", get(get_task))
        .route("/api/tasks/claim", post(claim_task))
        .route("/api/tasks/{id}/dispatch", post(dispatch_task))
        .route("/api/tasks/{id}/spawn", post(spawn_task))
        .route("/api/tasks/{id}/cancel", post(cancel_task))
        .route("/api/tasks/{id}/requeue", post(requeue_task))
        .route("/api/tasks/{id}/followup", post(followup_task))
        .route("/api/tasks/{id}/worktree", get(worktree_status))
        .route("/api/tasks/{id}/worktree/merge", post(worktree_merge))
        .route("/api/tasks/{id}/worktree/discard", post(worktree_discard))
        .route("/api/agents", get(list_agents))
        .route("/api/runners", get(list_runners))
        .route("/api/tasks/{id}/heartbeat", post(heartbeat_task))
        .route("/api/tasks/{id}/complete", post(complete_task))
        .route("/api/tasks/{id}/fail", post(fail_task))
        .route("/api/tasks/{id}/logs", post(add_task_log).get(get_task_logs))
        .route("/api/tasks/{id}/output", post(task_output))
        // Artifacts
        .route("/api/tasks/{id}/artifacts", post(create_artifact).get(list_artifacts))
        // Messages
        .route("/api/messages", post(send_message).get(get_messages))
        // SSE Events
        .route("/api/events", get(sse_events))
        // CORS 必須在驗證外層，瀏覽器的 preflight 才不會被擋
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), super::auth::require_token))
        .layer(super::auth::cors_layer())
        .with_state(state)
}

// Handlers
async fn create_task(
    State(state): State<ServerState>,
    Json(dto): Json<CreateTaskDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let task = TaskService::create_task(&state.pool, dto)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = state.event_tx.send(BoardEvent {
        event_type: "task_created".to_string(),
        payload: serde_json::to_value(&task).unwrap_or_default(),
    });

    Ok((StatusCode::CREATED, Json(task)))
}

#[derive(Deserialize)]
pub struct ListTasksQuery {
    pub q: Option<String>,
}

async fn list_tasks(
    State(state): State<ServerState>,
    Query(params): Query<ListTasksQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let tasks = TaskService::list_tasks(&state.pool, params.q.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(tasks))
}

async fn get_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let task = TaskService::get_task(&state.pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match task {
        Some(t) => Ok(Json(t)),
        None => Err((StatusCode::NOT_FOUND, "Task not found".to_string())),
    }
}

async fn claim_task(
    State(state): State<ServerState>,
    Json(dto): Json<ClaimTaskDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut task = TaskService::claim_task(&state.pool, &dto.agent_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    touch_runner(&state, &dto.agent_id, task.as_ref().map(|t| t.task.id.clone()));

    // 隔離執行：由伺服器建立 worktree，Runner 只需在回傳的 run_dir 執行
    if let Some(claimed) = task.as_mut() {
        if let Err(e) = worktree::prepare(&state.pool, claimed, &state.data_dir).await {
            let log = AddTaskLogDto { agent_id: dto.agent_id.clone(), level: Some("ERROR".into()), message: e };
            let _ = TaskService::add_log(&state.pool, &claimed.task.id, log).await;
            let _ = TaskService::fail_task(&state.pool, &claimed.task.id, &claimed.lease_id).await;
            let _ = state.event_tx.send(BoardEvent {
                event_type: "task_failed".to_string(),
                payload: serde_json::json!({ "id": claimed.task.id }),
            });
            return Ok(Json(None));
        }
    }

    if let Some(ref t) = task {
        // 廣播不含租約 ID，只有領取者拿得到
        let _ = state.event_tx.send(BoardEvent {
            event_type: "task_claimed".to_string(),
            payload: serde_json::to_value(&t.task).unwrap_or_default(),
        });
    }

    Ok(Json(task))
}

async fn dispatch_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(opts): Json<RunOptions>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let task = TaskService::dispatch_task(&state.pool, &id, &opts)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::CONFLICT, "Task not found or not queued".to_string()))?;

    let _ = state.event_tx.send(BoardEvent {
        event_type: "task_dispatched".to_string(),
        payload: serde_json::to_value(&task).unwrap_or_default(),
    });

    Ok(Json(task))
}

async fn spawn_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(opts): Json<RunOptions>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match state.spawner.spawn(&id, opts).await {
        Ok(task) => Ok(Json(task)),
        Err(SpawnError::UnsupportedAgent) => Err((StatusCode::BAD_REQUEST, "Unsupported agent".to_string())),
        Err(SpawnError::NotClaimable) => Err((
            StatusCode::CONFLICT,
            "Task not queued or its dependency is not completed".to_string(),
        )),
        Err(SpawnError::Busy) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tasks running locally; dispatch to a runner or try later".to_string(),
        )),
        Err(SpawnError::Worktree(e)) => Err((StatusCode::BAD_REQUEST, e)),
        Err(SpawnError::Db(e)) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn cancel_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cancelled = TaskService::cancel_task(&state.pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !cancelled {
        return Err((StatusCode::CONFLICT, "Task not queued or running".to_string()));
    }

    // 本機子程序立即 kill；Runner 執行的任務則在下次心跳被拒時自行中止
    state.spawner.kill(&id);
    let _ = state.event_tx.send(BoardEvent {
        event_type: "task_failed".to_string(),
        payload: serde_json::json!({ "id": id, "cancelled": true }),
    });
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Default)]
pub struct RequeueDto {
    /// true = 捨棄工作階段，從頭執行
    #[serde(default)]
    pub fresh: bool,
}

async fn requeue_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    body: Option<Json<RequeueDto>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let fresh = body.map(|Json(b)| b.fresh).unwrap_or(false);
    let result = TaskService::requeue_task(&state.pool, &id, fresh)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match result {
        RequeueResult::Requeued { downstream } => {
            let _ = state.event_tx.send(BoardEvent {
                event_type: "task_requeued".to_string(),
                payload: serde_json::json!({ "id": id, "downstream": downstream }),
            });
            Ok(Json(serde_json::json!({ "downstream": downstream })))
        }
        RequeueResult::NotFailed => Err((StatusCode::CONFLICT, "Task not found or not failed".to_string())),
        RequeueResult::DependencyFailed => Err((
            StatusCode::CONFLICT,
            "Its dependency has failed; requeue the dependency first".to_string(),
        )),
    }
}

#[derive(Deserialize)]
pub struct FollowupDto {
    pub prompt: String,
}

/// 追加指示：建立接續原任務工作階段的新任務（排隊中，由前端接著 spawn 或 dispatch）
async fn followup_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<FollowupDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if dto.prompt.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Prompt is empty".to_string()));
    }
    let task = TaskService::create_followup(&state.pool, &id, &dto.prompt)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::CONFLICT, "Task not found, not finished, or has no assigned agent".to_string()))?;
    let _ = state.event_tx.send(BoardEvent {
        event_type: "task_created".to_string(),
        payload: serde_json::to_value(&task).unwrap_or_default(),
    });
    Ok(Json(task))
}

/// 取得 worktree 操作的目標任務；執行中的任務不能合併或捨棄
async fn finished_task(state: &ServerState, id: &str) -> Result<crate::features::tasks::Task, (StatusCode, String)> {
    let task = TaskService::get_task(&state.pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Task not found".to_string()))?;
    if task.status == "running" {
        return Err((StatusCode::CONFLICT, "Task is still running".to_string()));
    }
    Ok(task)
}

async fn worktree_status(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let task = TaskService::get_task(&state.pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Task not found".to_string()))?;
    worktree::status(&task).await.map(Json).map_err(|e| (StatusCode::CONFLICT, e))
}

async fn worktree_merge(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let task = finished_task(&state, &id).await?;
    let message = worktree::merge(&state.pool, &task).await.map_err(|e| (StatusCode::CONFLICT, e))?;
    let _ = state.event_tx.send(BoardEvent {
        event_type: "worktree_changed".to_string(),
        payload: serde_json::json!({ "id": id }),
    });
    Ok(Json(serde_json::json!({ "message": message })))
}

async fn worktree_discard(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let task = finished_task(&state, &id).await?;
    worktree::discard(&state.pool, &task).await.map_err(|e| (StatusCode::CONFLICT, e))?;
    let _ = state.event_tx.send(BoardEvent {
        event_type: "worktree_changed".to_string(),
        payload: serde_json::json!({ "id": id }),
    });
    Ok(StatusCode::OK)
}

async fn list_agents() -> impl IntoResponse {
    Json(AGENTS)
}

async fn list_runners(State(state): State<ServerState>) -> impl IntoResponse {
    Json(state.runners.list())
}

/// 記錄 Runner 出現；狀態有變才廣播，閒置輪詢不會洗版
fn touch_runner(state: &ServerState, agent_id: &str, current_task: Option<String>) {
    if state.runners.touch(agent_id, current_task) {
        let _ = state.event_tx.send(BoardEvent { event_type: "runners_changed".to_string(), payload: serde_json::json!({ "agent_id": agent_id }) });
    }
}

/// 心跳 / 完成 / 失敗只帶任務 ID，從任務找出是哪個 Agent 的 Runner
async fn touch_runner_for_task(state: &ServerState, task_id: &str, running: bool) {
    if let Ok(Some(task)) = TaskService::get_task(&state.pool, task_id).await {
        if let Some(agent) = task.assigned_agent {
            touch_runner(state, &agent, running.then(|| task_id.to_string()));
        }
    }
}

const LEASE_MISMATCH: &str = "Task not running or lease mismatch";

async fn heartbeat_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<LeaseDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let success = TaskService::heartbeat(&state.pool, &id, &dto.lease_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if success {
        touch_runner_for_task(&state, &id, true).await;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::CONFLICT, LEASE_MISMATCH.to_string()))
    }
}

async fn complete_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<LeaseDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let success = TaskService::complete_task(&state.pool, &id, &dto.lease_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if success {
        touch_runner_for_task(&state, &id, false).await;
        let _ = state.event_tx.send(BoardEvent {
            event_type: "task_completed".to_string(),
            payload: serde_json::json!({ "id": id }),
        });
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::CONFLICT, LEASE_MISMATCH.to_string()))
    }
}

async fn fail_task(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<LeaseDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let success = TaskService::fail_task(&state.pool, &id, &dto.lease_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if success {
        touch_runner_for_task(&state, &id, false).await;
        let _ = state.event_tx.send(BoardEvent {
            event_type: "task_failed".to_string(),
            payload: serde_json::json!({ "id": id }),
        });
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::CONFLICT, LEASE_MISMATCH.to_string()))
    }
}

#[derive(Deserialize)]
pub struct OutputDto {
    pub lease_id: String,
    pub agent_id: String,
    pub lines: Vec<String>,
}

/// Runner 回傳 Agent 的原始輸出（stream-json 或純文字），由伺服器解析成日誌、結果與工作階段
/// 會寫入結果與工作階段，所以必須持有租約
async fn task_output(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<OutputDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let holds = TaskService::holds_lease(&state.pool, &id, &dto.lease_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !holds {
        return Err((StatusCode::CONFLICT, LEASE_MISMATCH.to_string()));
    }
    for line in &dto.lines {
        agents::ingest_output(&state.pool, &state.event_tx, &id, &dto.agent_id, line).await;
    }
    Ok(StatusCode::OK)
}

async fn add_task_log(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<AddTaskLogDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let log = TaskService::add_log(&state.pool, &id, dto)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = state.event_tx.send(BoardEvent {
        event_type: "log_added".to_string(),
        payload: serde_json::to_value(&log).unwrap_or_default(),
    });

    Ok((StatusCode::CREATED, Json(log)))
}

async fn get_task_logs(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let logs = TaskService::get_logs(&state.pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(logs))
}

async fn create_artifact(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(dto): Json<CreateArtifactDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let artifact = ArtifactService::create_artifact(&state.pool, &id, dto)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = state.event_tx.send(BoardEvent {
        event_type: "artifact_created".to_string(),
        payload: serde_json::to_value(&artifact).unwrap_or_default(),
    });

    Ok((StatusCode::CREATED, Json(artifact)))
}

async fn list_artifacts(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let artifacts = ArtifactService::list_by_task(&state.pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(artifacts))
}

async fn send_message(
    State(state): State<ServerState>,
    Json(dto): Json<SendMessageDto>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let msg = MessageService::send_message(&state.pool, dto)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = state.event_tx.send(BoardEvent {
        event_type: "message_sent".to_string(),
        payload: serde_json::to_value(&msg).unwrap_or_default(),
    });

    Ok((StatusCode::CREATED, Json(msg)))
}

async fn get_messages(
    State(state): State<ServerState>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let messages = MessageService::get_messages(
        &state.pool,
        &query.receiver,
        query.unread.unwrap_or(false),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(messages))
}

async fn sse_events(
    State(state): State<ServerState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, axum::Error>>> {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(event) => {
            let json = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().event(event.event_type).data(json)))
        }
        Err(_) => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn start_background_workers(
    pool: Pool<Sqlite>,
    event_tx: broadcast::Sender<BoardEvent>,
    lease_timeout_secs: i64,
) {
    // 超過 lease_timeout_secs 無心跳視為逾時；檢查頻率為逾時的 1/3（最多 10 秒一次）
    let check_every = (lease_timeout_secs / 3).clamp(1, 10) as u64;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(check_every));
        loop {
            interval.tick().await;
            if let Ok(recovered) = TaskService::recover_timed_out_tasks(&pool, lease_timeout_secs).await {
                if recovered.requeued + recovered.failed > 0 {
                    let _ = event_tx.send(BoardEvent {
                        event_type: "task_recovered".to_string(),
                        payload: serde_json::to_value(&recovered).unwrap_or_default(),
                    });
                }
            }
        }
    });
}

pub async fn start_http_server(
    port: u16,
    state: ServerState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let router = create_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
