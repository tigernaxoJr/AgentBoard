# 📋 AOA-TaskBoard (Agent Board)

基於 **Tauri v2 + Vue 3 (TypeScript) + Tailwind CSS** 與 **Rust Axum + SQLite (WAL)** 的多 Agent 自主協作看板系統。

支援 **Claude**（規劃與派發）與 **agy**（本機自主執行）的任務派發、依賴控制、狀態同步與即時日誌流。

---

## 🚀 快速上手 (Quick Start)

### 1. 安裝與啟動桌面端

#### 前置需求
* Node.js / `pnpm`
* Rust 工具鏈 (`rustc`, `cargo`)

#### 啟動開發環境
```bash
# 安裝前端依賴
pnpm install

# 啟動桌面應用 (自動啟動前端與內嵌 :18833 後端 API)
pnpm tauri dev
```

---

## 🤖 設定 Agent 常駐背景與「開機自動啟動」

為了讓 `agy` 能在沒有開啟桌面終端的情況下，**24 小時隨時自主領取工作**，推薦將 Runner 註冊為 Linux 使用者級 systemd 服務。

> [!TIP]
> 常駐的 Runner 僅是輕量 Python 腳本（佔用記憶體極小、0% CPU、**0 Token**），只有當看板出現工作並被領取時，才會在本機喚醒 `agy` session 執行！

### 一鍵啟用開機自啟動 (systemd --user)

執行以下指令，將服務設定複製到使用者服務目錄並啟用：

```bash
# 1. 建立使用者 systemd 設定目錄
mkdir -p ~/.config/systemd/user

# 2. 複製服務範本檔
cp scripts/agent-board-runner.service ~/.config/systemd/user/

# 3. 重新載入設定
systemctl --user daemon-reload

# 4. 啟用並立即啟動 Runner 服務
systemctl --user enable --now agent-board-runner.service

# 5. (選用) 允許即使使用者未登入 terminal，背景服務依然在開機後持續運行
loginctl enable-linger $USER
```

### 常用管理指令

* **查看 Runner 運行狀態與日誌**：
  ```bash
  systemctl --user status agent-board-runner.service
  journalctl --user -u agent-board-runner.service -f
  ```
* **重啟 Runner 服務**：
  ```bash
  systemctl --user restart agent-board-runner.service
  ```
* **停止與關閉自啟動**：
  ```bash
  systemctl --user stop agent-board-runner.service
  systemctl --user disable agent-board-runner.service
  ```

---

## 🔄 手動測試 Runner (非 Daemon 模式)

如果只想臨時手動測試 `agy` 領單與執行：

```bash
# 以乾跑模式 (Dry-run 模擬執行與回報日誌，不實際調用 agy)
agent-board runner --agent agy --dry-run

# 以真實模式啟動
agent-board runner --agent agy --interval 3

# 同時執行多個任務（預設 1）
agent-board runner --agent agy --concurrency 3
```

---

## 🧪 端對端測試

```bash
node scripts/e2e/e2e.mjs
```

用假的 Agent 起真的 server 與 Runner，驗證任務生命週期、失敗與重試、斷線恢復等 30 個情境（需要 cargo 與 rustc，不需要真的 claude / agy）。

## 🧩 範本、統計與 Diff API

| 路徑 | 說明 |
| --- | --- |
| `GET/POST /api/templates` | 列出 / 建立任務範本 |
| `GET/PUT/DELETE /api/templates/{id}` | 取得 / 更新 / 刪除範本（更新時選填欄位傳空字串 = 清空） |
| `GET /api/stats?from=YYYY-MM-DD&to=YYYY-MM-DD` | 花費、Token、耗時統計（`to` 含當天；`by_date` 為升冪） |
| `GET /api/cli-sessions` | 列出本機 claude 既有 session（含非看板建立的）；建立任務時帶 `session_id` + `workdir` 即可接續 |
| `GET /api/tasks/{id}/worktree/diff?path=` | worktree 相對 base commit 的完整 diff（含未追蹤新檔案，不修改 index） |

建立任務時可用 `dependency_ids` 傳多個前置任務：全部完成才會解鎖，任一失敗則連鎖失敗。

---

## 🔌 MCP Server 支援 (Model Context Protocol)

Agent Board 執行檔本身即可兼當原生 MCP 伺服端（`agent-board mcp`），採用官方 `rmcp` 核心與 stdio 傳輸，**完全不需要安裝 Python 或外部套件**。
讓 Claude Code、Claude Desktop、agy 或任何支援 MCP 的 Agent 能直接以原生工具呼叫看板，不必再寫 curl。若 Agent Board 主程式尚未開啟，工具亦會友善回傳提示。

### 支援工具清單
* `list_tasks`：搜尋/檢視任務清單
* `get_task`：取得任務詳情、結果、日誌與 Token
* `create_task`：建立新任務（支援 model, workdir, dependency_ids（多重依賴）, isolate, requires_approval 等；`requires_approval: true` 的任務先停在「待核准」，要人類在看板核准才會排入佇列。Agent 在執行任務時建立的任務會累計派發深度，超過 5 層即拒絕，避免互相派發的迴圈）
* `spawn_task`：由看板立即啟動子程序執行
* `dispatch_task`：排入待辦派工給 Runner
* `followup_task`：追加後續指示（接續 session）
  * 每個任務的 prompt 後會附上中性的說明（不提平台、任務 id 或派發者）：請 Agent 把產出檔案存到 `./outputs/<任務 id 的雜湊>/`，任務完成時平台自動把該資料夾內的檔案登記為成果，Agent 不需要 MCP 也不需知道 task_id；未指定工作目錄的任務，平台會在資料目錄下配 `workspaces/<id 的雜湊>/` 當工作目錄，確保輸出資料夾位置確定
  * Agent 若在結果結尾附上 ```` ```ask ```` 區塊（問題與選項），UI 會顯示成按鈕供點選回答；任務詳情也有「壓縮對話」（`/compact`）按鈕
* `requeue_task` / `cancel_task`：重試或取消
  * 同一個 Agent 連續 3 次實際執行都失敗時，Runner 會暫停領取新任務（側邊欄 Runners 顯示「已暫停・恢復」，排除問題後按一下恢復；取消與連鎖失敗不計入）
* `add_artifact` / `list_artifacts`：登記或檢視產出成果檔案（圖片會在任務抽屜顯示縮圖，點擊放大；檔案須在任務工作目錄內）
* `list_agents` / `list_runners`：查詢可用 Agent 與 Runner
* `send_message` / `read_messages`：Agent 間訊息（收件者 `*` = 廣播給其他所有人）；`read_messages` 預設只取未讀並標為已讀，廣播訊息每個接收者各自記錄已讀
* `wait_task`：等待任務結束（最多 300 秒，逾時回傳目前狀態並標示 `timed_out`），取代自己反覆輪詢 `get_task`

### 配置方式範例

#### 1. 桌面端一鍵安裝（最推薦）
開啟 Agent Board 桌面端 ➜ 點擊右上角「接入指南」➜ 點擊 **`⚡ 一鍵加入 Claude`** 或 **`⚡ 一鍵加入 agy`**，系統將自動為您完成配置！

#### 2. CLI 手動新增
```bash
claude mcp add agent-board /path/to/agent-board mcp
# 或 agy
agy mcp add agent-board /path/to/agent-board mcp
```

#### 3. Claude Desktop (`claude_desktop_config.json`)
```json
{
  "mcpServers": {
    "agent-board": {
      "command": "/path/to/agent-board",
      "args": ["mcp"]
    }
  }
}
```
*(MCP Server 會自動從系統預設目錄讀取 `api_token`，亦支援 `AGENT_BOARD_TOKEN` 環境變數)*

---

## 📁 Agent 檔案交換規範 (File Exchange)

多個 Agent 之間需要交接成果檔案時，依循以下機制：

1. **約定工作目錄**：
   產出檔案存放於專案本機工作區：`./workspace/tasks/{task_id}/`。
2. **登記 Artifact**：
   執行端（如 `agy`）完成任務後，呼叫：
   ```http
   POST /api/tasks/{task_id}/artifacts
   Content-Type: application/json

   {
     "agent_id": "agy",
     "file_path": "./workspace/tasks/{task_id}/result.json",
     "summary": "分析結果報表"
   }
   ```
3. **完成任務**：
   呼叫 `POST /api/tasks/{task_id}/complete`。
4. **下游 Agent 讀取成果**：
   依賴該任務的下游 Agent（如 `claude`）領取下游任務後，呼叫 `GET /api/tasks/{前置task_id}/artifacts` 取得檔案路徑，即可直接在本機讀取該檔案接續作業。

---

## 📦 發行版本體積優化 (Release Build)

本專案已在 `src-tauri/Cargo.toml` 配置深度的二進制瘦身優化：
* 啟用 Fat LTO (`lto = "fat"`)
* 最小二進制優化 (`opt-level = "z"`)
* 符號表自動剝除 (`strip = true`)
* 單碼生成單元 (`codegen-units = 1`)
* Panic 直接中斷 (`panic = "abort"`)

編譯打包指令：
```bash
pnpm tauri build
```
產出的二進制可執行檔與安裝套件將具備最小化體積。
