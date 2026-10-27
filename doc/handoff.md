# 🤝 交接文件

最後更新：2026-10-27（最新 commit `72e91f9`）

已完成的功能與實作備註見 [todo.md](todo.md)，架構規劃見 [plan.md](plan.md)、[hybrid_execution_plan.md](hybrid_execution_plan.md)。這份只列**還沒做的事**與接手需要知道的狀態。

## 目前狀態

- 分支 `main`，**沒有設定 git remote**，所有 commit 都只在本機。
- 工作目錄有一個**未 commit 的改動**：`src-tauri/Cargo.toml` 的 `[profile.release]` 把 `codegen-units` 從 1 改成 16（不是開發過程中改的，請確認是否保留）。
  - 1：執行檔最小，但 release 最後的最佳化與連結只能單核心跑（8 核心平均只用到約 150% CPU，約 2 分 36 秒）。
  - 16：編譯快很多，執行檔稍大。若同時把 `lto = "fat"` 改成 `"thin"` 會更快。
- `scripts/__pycache__/runner.cpython-314.pyc` 被 git 追蹤了，每次執行或 `py_compile` runner 都會變動。建議 `git rm --cached` 並加進 `.gitignore`。
- release 執行檔（`pnpm tauri build --no-bundle` → `src-tauri/target/release/agent-board`）有成功建置過，但**沒有實際啟動測試過**，而且是在最近兩個 commit 之前建的，需要重新建置。

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
