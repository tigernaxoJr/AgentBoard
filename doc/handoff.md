# 🤝 交接文件

最後更新：2026-10-27（最新 commit `72e91f9`）

已完成的功能與實作備註見 [todo.md](todo.md)，架構規劃見 [plan.md](plan.md)、[hybrid_execution_plan.md](hybrid_execution_plan.md)。這份只列**還沒做的事**與接手需要知道的狀態。

## 目前狀態

- 分支 `main`，**沒有設定 git remote**，所有 commit 都只在本機。
- release 執行檔（`pnpm tauri build --no-bundle` → `src-tauri/target/release/agent-board`）支援 `lto = "thin"` 高速並行編譯。
- 已加入 `AGENTS.md`，明確規定所有 Agent 實作功能時必須同步更新文件。

## 已完成的主要功能 (Completed)

- [x] **多個前置任務 (Multi-dependency DAG)**：完全改用 `task_dependencies`，支援原子檢查全部前置完成、連鎖失敗/恢復、多結果 Prompt 串接。
- [x] **MCP Server 支援**：提供原生 stdio 腳本 `scripts/mcp_server.py`，支援 13 個工具操作看板。
- [x] **Token / 花費統計面板**：`GET /api/stats` 與前端 `StatsModal.vue`（按 Agent、工作目錄、日期）。
- [x] **任務範本庫 (Task Templates)**：`task_templates` 表、CRUD API、`TemplateModal.vue`，支援任務一鍵存為範本與瀏覽工作目錄。
- [x] **Worktree 完整 Diff 檢視**：`GET /api/tasks/{id}/worktree/diff` 與抽屜內嵌完整 diff 檢視器。
- [x] **任務刪除功能**：`DELETE /api/tasks/{id}`，抽屜加入「🗑️ 刪除」按鈕，連帶清理相關日誌與依賴。
- [x] **原生地圖/資料夾選擇彈窗**：整合 `tauri-plugin-dialog`，在建立任務與範本庫可直接彈窗瀏覽選目錄。
- [x] **模型下拉選單與前置勾選**：建立任務彈窗支援直接勾選已有任務，並依 Agent 自動帶出可用模型下拉選單。
- [x] **任務快速複製 (Duplicate)**：抽屜一鍵複製當前任務內容並打開建立彈窗。
- [x] **Running 即時執行碼表**：看板卡片與抽屜即時顯示 running 任務的動態累計秒數與呼吸燈。
- [x] **Agent CLI 更新檢查 (CLI Updater)**：後端 `POST /api/agents/{id}/update` 整合 `agent --version` 與 `agent update`，前端提供「🚀 CLI 更新」彈窗支援一鍵檢查與更新 `agy` 與 `claude` CLI。
- [x] **Agent 本地額度與用量監控 (Zero-Token CLI Usage)**：後端 `GET /api/agents/usage` 直接讀取本機 CLI 快取與日誌，前端側邊欄即時呈現 Claude 5h/7d 額度條、已用金額與 Agy 今日 Session/Token 計量，查詢過程完全不消耗任何 LLM Token。
- [x] **Tauri System Tray 系統匣與生圖圖示**：透過生圖模型產製 2x2 Spritesheet，切割出全新 App Icon 與三種 System Tray 狀態圖示（`tray-idle`、`tray-active`、`tray-alert`），並完成 Rust System Tray 托盤選單與點擊視窗最小化/還原控制。
- [x] **指南動態路徑與 Token 解析**：在 `get_api_config` 加入 `project_dir` 與 `token_file_path`，`ApiGuideModal` 會自動帶入使用者當前執行專案的真實絕對路徑與實際 Token，不再需要手動替換 `/path/to/agent_board`。
- [x] **Agent 自我描述規格端點 (`GET /api/spec`)**：提供免 Token 驗證的精簡 Markdown 規格說明書，讓終端 AI 可透過單一 curl 一秒讀懂發派、輪詢、DAG 串接與結果回傳；「接入指南」彈窗首推一鍵複製 AI Prompt。
- [x] **接入指南極簡化與手冊折疊 (Collapsible API Reference)**：指南只保留「方式一：/api/spec 貼給 AI」與「方式二：MCP 工具」兩大核心方式；下方 API Reference 改為清爽折疊列表，點擊即可展開該端點完整的 curl 範例指令與複製。
- [x] **一鍵安裝 MCP Server 至 Agent**：在接入指南加入 `⚡ 一鍵加入 Claude` 與 `⚡ 一鍵加入 agy` 按鈕，後端透過 Tauri Command `install_mcp_server` 自動執行 `claude mcp add` / `agy mcp add` 將看板註冊進使用者的 CLI 工具。
- [x] **原生 Rust 兼當 MCP Server (`agent-board mcp`)**：採用官方 `rmcp` 3.8.0 實作 MCP stdio 協定，直接以執行檔自身（如 `agent-board.exe mcp`）提供 stdio 服務，無需外部 Python 依賴。內建 13 個 MCP 工具轉發至本地 `:18833`，若 Agent Board 尚未啟動會友善回傳提示，桌面端一鍵安裝與複製指令全面改為原生子命令。
- [x] **執行中即時插話 / 中斷轉向 (Mid-turn Interrupt & Follow-up)**：在任務執行中（running）時抽屜輸入框不再鎖定，使用者隨時可打字發送插話；後端收到時會優雅終止原任務子程序，並帶著累積的 session_id 即時喚醒新任務接續對話，達成隨時插話的效果。

## 待辦功能（依建議優先序）

### 1. 多個前置任務
目前一個任務只能有一個 `depends_on_task_id`。

> **已完成並移除舊欄位**：已改用 `task_dependencies` 表，且 `tasks.depends_on_task_id` 已刪除，API 只接受 `dependency_ids`。
> 原因：多重依賴後，單一欄位與依賴表並存會有兩個真相來源，必須同步維護，也讓 claim、連鎖失敗與重排的查詢更難推理。專案尚無線上版本，不需要相容舊 API 與舊資料，所以直接根本移除，不保留「主要依賴」。下方「做法」中保留舊欄位與搬資料的部分已不適用。

- 做法：新增 `task_dependencies(task_id, depends_on_task_id)` 表（或 JSON 欄位），保留舊欄位做相容並在 migration 搬資料。
- 需要一起改的地方：
  - `TaskService` 兩個 claim 查詢的「前置已完成」條件改成「全部前置都 completed」。
  - `cascade_dependency_failures` 與 `requeue_task` 的遞迴 CTE（任一前置失敗就連鎖失敗；重排時恢復下游）。
  - `ClaimedTask::render_prompt`：`{{dep.result}}` 串接全部前置結果（各自加標題），或支援 `{{dep.<短ID>.result}}` 指定某一個。
  - 前端：`CreateTaskModal` 可輸入多個 ID；`DependencyGraph.vue` 已經是「每條依賴一條邊」，改成讀陣列即可；`blockedLabel` 改成檢查全部前置。
  - `ApiGuideModal` 的範例與說明。

### 2. MCP server
讓 Claude 等 Agent 以工具呼叫看板，不必在 prompt 裡教它打 curl。
- 工具建議：`create_task`（含 depends_on、agent、model）、`list_tasks`（含搜尋）、`get_task`（含結果）、`dispatch_task` / `spawn_task`、`add_artifact`、`send_message`。
- 做法選項：
  - 在 Rust 後端加一個 stdio MCP 模式（例如 `agent-board mcp`），內部轉呼叫本機 HTTP API，token 從 App 資料夾讀（與 `runner.py` 相同邏輯）。
  - 或寫成獨立的 Python / Node 小程式，只包 HTTP API，最省事。
- 接入指南加一段「`claude mcp add agent-board ...`」的設定方式。

### 3. 花費統計面板
資料都已經在 `tasks` 表（`cost_usd`、`input_tokens`、`output_tokens`、`duration_ms`、`assigned_agent`、`workdir`、`updated_at`）。
- 做法：`GET /api/stats?from=&to=` 依日期 / Agent / 工作目錄彙總；前端在標題列的「看板 / 關係圖」再加一個「統計」頁。
- 注意：agy 不回報費用，只能顯示 token 與耗時。

### 4. 任務範本
存常用的「Prompt + Agent + 模型 + 權限 + 隔離」組合，一鍵建立任務。
- 做法：`task_templates` 表與 CRUD API；`CreateTaskModal` 加「從範本建立 / 存成範本」。Prompt 可沿用 `{{dep.result}}`。

### 5. worktree 完整 diff 檢視
目前抽屜只顯示 diff 統計（`WorktreePanel.vue`），看不到內容。
- 做法：`GET /api/tasks/{id}/worktree/diff?path=` 回傳單一檔案的 `git diff <base_commit>`；前端逐檔展開顯示。
- 順便處理已知限制：diff 統計不含未加入 git 的新檔案（可用 `git add -N` 或 `git diff --no-index` 補上）。

### 6. 排程 / 定期任務
例如每天早上建立一個「檢查依賴更新」任務。
- 做法：`schedules` 表（cron 字串 + 範本），後端背景工作每分鐘檢查並建立任務。可建立在「任務範本」之上，建議在 4 之後做。

## 其他可改進（較小）

- `/output` 端點逐行呼叫 `ingest_output`，每行一次 DB 寫入；Runner 已批次送出，伺服器端可再包成單一 transaction。
- 日誌修剪是「全域每 100 次寫入修剪一次目前任務」，很少寫入的任務可能稍微超過 2000 則上限；抽屜也還不能往前載入較早的日誌。
- 任務列表與搜尋都最多回傳 200 筆已結束任務，尚無完整分頁或封存。
- Runner 在線狀態只存在記憶體，面板重啟後要等 Runner 下次輪詢才出現；同一個 Agent 有多個 Runner 時合併成一筆。
- 逾時自動重試一律接續上一段對話（手動重排才能選從頭開始）。

## 開發與測試方式

```bash
# Rust 單元測試（目前 48 個）
cd src-tauri && cargo test

# 前端型別檢查與建置
pnpm build

# 不開視窗只跑 API（測試 Runner 用），DB 與 token 放在任意測試資料夾
cd src-tauri && PORT=18833 LEASE_TIMEOUT_SECS=30 cargo run --example headless -- /path/to/board.db /path/to/api_token

# Runner 處理完佇列就結束
python3 scripts/runner.py --agent agy --token "$(cat /path/to/api_token)" --once

# 前端單獨在瀏覽器開（不經 Tauri 時用環境變數給 token）
VITE_API_TOKEN="$(cat /path/to/api_token)" pnpm dev   # http://localhost:1420
```

- 端對端驗證過的流程：claude / agy 的 spawn 與 Runner 派發、worktree 隔離、追加指示接續工作階段、`{{dep.result}}` 串接、搜尋、從頭重試。
- agy 的 `-p` 會把下一個參數當 prompt，所有 flag 必須放在 `-p` 前面（`features/agents/mod.rs` 的 `command_for`）。
- agy 的 result 事件裡的 usage / duration 是整段對話的累計，用量改由每個完成步驟加總（`features/agents/stream.rs`）。
