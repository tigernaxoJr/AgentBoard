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

# 啟動桌面應用 (自動啟動前端與內嵌 :3000 後端 API)
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
python3 scripts/runner.py --agent agy --dry-run

# 以真實模式啟動
python3 scripts/runner.py --agent agy --interval 3
```

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
