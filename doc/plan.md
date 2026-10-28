# 📋 AOA-TaskBoard 專案實作計畫

## 🎯 專案核心目標
1. **Pull 模式自主領取**：Claude / 人類只負責將任務丟進中心看板（`status: queued`），由 `agy` 或其他 Agent 自主 Polling 領取（`claimed` / `running`），符合政策規範。
2. **多 Agent 協作與狀態同步**：支援 Agent 之間透過 REST API 或共享檔案系統（Filesystem）進行狀態回報、交接與成果上傳。
3. **可視化看板與手動控制**：提供 Web UI 介面，支援拖曳/按鈕派發（Play）、複製提示詞（Copy Prompt），並即時掌握各 Agent 進度。

---

## 🛠️ 技術架構選型
* **桌面應用框架**：**Tauri (v2)** —— 結合 Rust 原生效能與極小體積的跨平台桌面端。
* **後端核心 (Tauri Core / Rust)**：
  * 架構模式：**VSA (Vertical Slice Architecture) + 輕量 DDD (Domain-Driven Design)**。
    * 遵循「針對業務功能進行縱向切片（Feature Slice），高內聚管理」。
    * **模組過小的功能不進行過度 DDD 抽象**（如直接 Transaction Script / CRUD，避免為了 DDD 而生硬拆出多餘的 Aggregate、Value Object 或 Repository 介面）；僅在任務狀態機與租約流轉（Task Lifecycle & Lease）等具備複雜業務規則的核心處套用領域模型。
  * 資料庫：`SQLx` + SQLite（WAL 模式支援多程序/Agent 高並發讀寫）。
  * 外部通訊服務：內嵌輕量 HTTP 伺服器（`Axum`）供外部 Agent（如 Claude / `agy` Runner）呼叫 REST API。
* **前端 (Tauri Webview)**：
  * 套件管理：`pnpm`
  * 框架：**Vue 3 (Composition API) + TypeScript**
  * 樣式系統：**Tailwind CSS**
  * 狀態與通訊：Tauri IPC Events（桌面內部通訊） + 原生 SSE（即時看板更新）。
* **通訊層**：
  * **外部 Agent 任務/狀態控制**：REST API (`/api/tasks`) 與自我描述規格 (`/api/spec`)
  * **桌面內部 UI 與 Rust 核心**：Tauri Commands & Events
  * **成果資料夾**：約定本地工作資料夾（預設 `./workspace/tasks/{id}/`）。

---

## 📅 開發階段規劃

### Phase 1：專案架構建立與核心後端 (Tauri + Rust + VSA)
* **目標**：以 Tauri (v2) 初始化專案，整合 `pnpm` 前端骨架，並依 VSA + 輕量 DDD 建立後端功能切片與 SQLite 資料庫。
* **執行項目**：
  1. 使用 Tauri 初始化專案骨架（前端 Vue 3 + TypeScript + Tailwind CSS，套件管理使用 `pnpm`）。
  2. 後端架構採用 **VSA (Vertical Slice Architecture)** 分割模組：
     * `features/tasks/`：任務管理（核心狀態機使用輕量 DDD 管理 status 流轉與 heartbeat 租約，其餘簡易查詢直接以 query handler 處理）。
     * `features/artifacts/`：成果上傳（小型模組，簡單 CRUD / Transaction Script 即可，不進行過度 DDD 抽象）。
     * `features/messages/`：Agent IPC 訊息中心（小型模組，輕量實作）。
     * `server/`：內嵌 Axum HTTP 服務，暴露 REST API 供外部 Agent 呼叫。
  3. 設定 `SQLx` 與 SQLite (啟用 WAL 模式以支援多 Agent 同時讀寫)。
  4. 建立資料表：

> **後續變更（已移除 `tasks.depends_on_task_id`）**：依賴改由 `task_dependencies(task_id, depends_on_task_id)` 表表示，API 只接受 `dependency_ids` 陣列。
> 原因：單一欄位只能表達一個前置任務；改成多重依賴後，兩處並存會產生兩個真相來源，必須同步維護，也讓 claim、連鎖失敗與重排的查詢更難推理。
> 專案尚無線上版本，不需要相容舊 API 與舊資料，因此直接移除欄位，不保留「主要依賴」。下方內容為當時的原始規劃。
     * `tasks` (`id`, `title`, `prompt`, `status`, `assigned_agent`, `creator`, `depends_on_task_id`, `last_heartbeat_at`, `created_at`, `updated_at`)
     * `artifacts` (`id`, `task_id`, `agent_id`, `file_path`, `summary`, `created_at`)
     * `task_logs` (`id`, `task_id`, `agent_id`, `level`, `message`, `created_at`)
     * `agent_messages` (`id`, `sender`, `receiver`, `payload`, `read_status`, `created_at`)
  5. 實作 REST API 端點：
     * `POST /api/tasks`：建立新任務（Claude / 人類寫入，支援多重前置依賴 `dependency_ids`、指定 `model`、`workdir`、`isolate`、`auto_approve`）。
     * `GET /api/tasks/{id}`：取得單一任務詳情。
     * `DELETE /api/tasks/{id}`：刪除任務及其依賴、產出、日誌。
     * `POST /api/tasks/claim`：**原子性領取 API**，避免並發搶單衝突（單一操作完成查詢就緒任務並標記 `running`）。
     * `GET /api/tasks`：查詢任務清單與狀態。
     * `POST /api/tasks/{id}/heartbeat`：Agent 定期回報心跳，維持租約（Lease）。
     * `POST /api/tasks/{id}/logs`：即時追加執行日誌（並透過 SSE 推播）。
     * `GET /api/artifacts/{id}/file`：讀取成果檔案內容（路徑須落在任務的 workdir / worktree 內、上限 20MB；圖片依副檔名給 image/* 其餘為 octet-stream）。UI 以帶 token 的 fetch 取 blob 顯示縮圖，點擊放大。
     * `POST /api/tasks/{id}/complete`：Agent 回報完成並上傳成果路徑。
     * `POST /api/tasks/{id}/fail`：Agent 回報失敗或錯誤資訊。
     * `GET /api/stats`：統計面板資料彙整 API（按 Agent、工作目錄、日期）。
     * `/api/templates`：任務範本 CRUD API。
     * `GET /api/tasks/{id}/worktree/diff`：隔離執行的完整 Git Diff API。
     * `POST /api/agents/{id}/update`：檢查並更新 Agent CLI（支援 `agy`、`claude`），回傳舊版本、新版本與執行日誌輸出。
     * `GET /api/cli-sessions`：掃描 `~/.claude/projects/*/*.jsonl`（只讀檔頭 256KB）列出本機 claude 既有 session（含非看板建立的），依最後修改時間排序、最多 200 筆；無法取得 cwd 的略過。`POST /api/tasks` 的 `session_id` 欄位可直接帶入，任務執行時以 `--resume` 接續。
     * `GET /api/agents/usage`：零 Token 消耗查詢各 Agent CLI 本地用量與額度（Claude 5h/7d 限額快取、Agy 今日 Tokens 累計）。
  6. 加入背景定時檢查：
     * 自動偵測 `last_heartbeat_at` 逾時的任務，將其恢復為 `queued` 或標記 `failed`。
  7. 加入 **SSE (`GET /api/events`)** 讓後端狀態變更與日誌即時推播。
  8. **失敗原因與重試退避**：`tasks.error` 記錄最近一次失敗的原因（程序結束碼、無法啟動、Agent 自己回報失敗、租約逾時、取消、前置任務失敗 / 被刪除），重新領取或重新排隊時清除；`POST /api/tasks/{id}/fail` 可附 `reason`。逾時放回佇列時設定 `tasks.not_before`（第 n 次嘗試後等 `n × 30` 秒），領取時略過尚未到期的任務；`spawn`（面板直接執行）不受退避限制。Agent 的結果事件帶 `is_error`（claude）或非 `SUCCESS`（agy）時，任務直接標記失敗，即使程序結束碼是 0，避免下游任務拿錯誤的結果繼續執行。
  9. **資料庫版本**：舊的「缺欄位就補」邏輯視為版本 0 不再擴充；之後的結構變更加進 `db::MIGRATIONS`，依序各執行一次，版本號存在 `PRAGMA user_version`。

### Phase 2：Agent 端介接與測試（先期：輪詢機制）
* **目標**：讓派發任務的 Agent 與執行的 `agy` 透過輪詢機制無縫對接看板。
* **執行項目**：
  1. **執行端（`agy` 接收任務）**：
     * 撰寫外部常駐 Runner 腳本，以定時輪詢方式呼叫 `POST /api/tasks/claim?agent_id=agy` 原子領取任務。
     * 領到任務後，由本機 Runner 喚醒全新的獨立 `agy` process/session 執行，避免常態空轉消耗 Token。
     * 執行期間由 Runner 背景回報 heartbeat，並建立本地工作目錄（預設 `./workspace/tasks/{task_id}/`）。
     * 執行完畢後由 Runner 呼叫 `complete` / `fail` 並上傳 artifacts，隨後關閉該 session。
  2. **派發端（如 Claude / 派工 Agent 追蹤狀態）**：
     * 派工端建立任務後，透過定期輪詢 `GET /api/tasks/{task_id}` 確認任務執行狀態（`running`、`completed`、`failed`）與成果路徑。
  3. 實作 Agent 間通訊機制：
     * 支援 Agent 透過 `POST /api/messages` 互傳訊息（`sender`、`receiver`、`payload`、可選 `task_id`；`receiver = *` 為廣播，不會送給發送者自己）。
     * `GET /api/messages?receiver=<id>&unread=true&task_id=&limit=` 取得收件匣（含他人的廣播）；`POST /api/messages/read {reader, ids?}` 標記已讀（省略 `ids` = 全部）。已讀紀錄存在 `message_reads(message_id, reader)`，每個接收者各自一份，所以廣播不會被一個人讀過就對其他人消失；舊 `agent_messages.read_status` 欄位已不使用。
     * `GET /api/messages/all?limit=` 為看板檢視（全部訊息，`is_read` 為收件者本人是否已讀），UI「訊息」彈窗使用，並可以 `human` 身分發送。
     * 失控防護：`tasks.depth` 記錄 Agent 再派發 Agent 的層數（建立者是某 Agent 時 = 它執行中任務的最大 depth + 1，人類 = 0），超過 `MAX_DEPTH`（5）回 422；同一 Agent 最近 `PAUSE_AFTER_FAILURES`（3）筆實際執行的任務全失敗即暫停領取（`claim_task` 回傳空；取消與 attempts = 0 的連鎖失敗不計），`POST /api/agents/{id}/resume` 寫入 `agent_resume.resumed_at` 後重新計數，`GET /api/agents/paused` 列出被暫停者。
     * 人類審批：建立任務時 `requires_approval: true` → 狀態 `pending_approval`（無法被領取 / 派發 / 執行），`POST /api/tasks/{id}/approve` 轉為 `queued`；可取消，前置任務失敗時一併連鎖失敗。只有 Agent 自願標記才會觸發，平台不強制。
     * MCP：`send_message` / `read_messages`；派發端可用 `wait_task` 等待結果（MCP 端每 2 秒檢查一次，最多 300 秒），不必自己輪詢。

### Phase 3：前端看板介面 (Vue 3 + Tailwind CSS)
* **目標**：打造直覺的任務管理介面，掌握全局。
* **執行項目**：
  1. 建立 Kanban 檢視（欄位：`Queued` / `Running` / `Completed` / `Failed`）。
  2. 任務卡片功能：
     * 顯示任務標題、建立者（Claude / 人類）、指定 Agent。
     * **「Play」按鈕**：手動強制觸發指定 Agent 執行。
     * **「Copy Prompt」按鈕**：一鍵複製完整提示詞，方便手動貼給無法直接連網或 API 溝通的 Agent。
     * 檢視工作成果與檔案路徑。
  3. 整合前端 SSE 監聽，讓看板無須重新整理即可動態即時更新。

### Phase 4：整合測試與閉環驗證
* **目標**：驗證 Claude 丟任務 $\rightarrow$ 看板 $\rightarrow$ `agy` 自動領取 $\rightarrow$ 派發端輪詢結果的完整閉環。
* **執行項目**：
  1. 模擬 Claude 透過 API 建立任務並輪詢等待結果。
  2. 啟動 `agy` Runner 進行背景輪詢與自主領取。
  3. 檢查 SQLite 效能與並發鎖定狀況（確認 WAL 模式運作正常）。
  4. 調整錯誤處理（如 Agent 逾時未回應的狀態恢復）。

### Phase 5：進階優化 —— SSE / Webhook 事件喚醒架構（未來規劃）
* **目標**：將現有的外部輪詢全面升級為事件驅動喚醒，達成真正的 0 輪詢負載與毫秒級即時響應。
* **執行項目**：
  1. **事件推播管線完善**：看板後端透過 SSE 頻道（`GET /api/events`）廣播 `task_ready` 與 `task_updated` 事件。
  2. **事件喚醒 Runner**：外部 Runner 掛起監聽 SSE，收到事件通知時立即喚醒本機開新 `agy` session，完全取代定時輪詢。
  3. **派發端事件通知**：派發端可選擇透過監聽 SSE 串流或接收 Webhook 回呼取代輪詢確認任務狀態。

### Phase 6：多 Agent 身分與存取控制（未來規劃）
> 目前定位是**本機單人使用**，所有 Agent 共用同一個 API token，風險有限，所以先不做；之後要開放給多人或遠端 Runner 時再處理。
* **目標**：讓每個 Agent 只能做自己該做的事，避免冒充與越權。
* **已知缺口**：
  1. **身分不可信**：`agent_id`、`creator`、訊息的 `sender` 都是呼叫端自己填的，持有 token 的任何一方都能冒充別人（領取、寫日誌、登記成果、發訊息）。`complete` / `fail` / `heartbeat` 已經要求租約，不受影響。
  2. **`auto_approve` 搭配任意 `workdir`**：任何持有 token 的呼叫端都能建立「完整權限、在任意目錄執行」的任務。
* **執行項目**：
  1. 每個 Agent（Runner / MCP）各自一組 token，token 對應固定的 `agent_id`；伺服器以 token 決定身分，忽略請求裡自報的 `agent_id` / `sender` / `creator`。看板視窗保留一組 human token。
  2. 可設定允許的 `workdir` 根目錄清單；範圍外的路徑不能建立任務，`auto_approve` 可另外限定只在清單內有效。
  3. Runner、MCP 安裝流程與「API 說明」彈窗要改成配發 / 顯示各自的 token。
* **決策待定**：根目錄清單存放位置（App 設定檔或資料庫）、舊的單一 token 是否保留為 human 專用。

