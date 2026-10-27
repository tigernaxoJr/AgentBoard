# 📝 待辦功能

依建議優先序排列。完成後打勾並註明 commit。

## 小（補上目前流程的缺口）

- [x] **失敗任務「重新排隊」**
  目前 `failed` 是終點，重跑只能重建任務，依賴它的任務也一起卡死。
  做法：`POST /api/tasks/{id}/requeue` 把任務（以及因連鎖失敗的下游任務）改回 `queued`、`attempts` 歸零；抽屜加按鈕。

- [x] **顯示成果（artifacts）**
  後端已有 `POST/GET /api/tasks/{id}/artifacts`，UI 尚未顯示。
  做法：抽屜新增「成果」區塊，列出檔案路徑與摘要；用 `tauri-plugin-opener` 一鍵開啟檔案或工作目錄。
  實作備註：只提供「在檔案管理器中顯示」，不直接開啟檔案（路徑由 Agent 回報，直接開啟可能執行到腳本）。

- [ ] **桌面通知**
  任務完成 / 失敗時跳系統通知，不必一直盯著看板。
  做法：`tauri-plugin-notification`，在前端收到 `task_completed` / `task_failed` 事件時通知；設定可關閉。

## 中

- [x] **每個任務一個 git worktree**
  多個 coding agent 同時修改同一個 repo 會互相覆蓋。
  做法：任務新增「隔離」選項，執行前在 `workdir` 的 repo 建立 `agent-board/<task-id>` 分支與 worktree，Agent 在 worktree 內執行；完成後在抽屜顯示 diff 摘要與「合併 / 刪除」操作。

- [ ] **延續對話（追加指示）**
  任務完成後可在抽屜補一句「再補上測試」，接續同一個 Agent session。
  做法：記錄 session / conversation ID（claude `--output-format json` 回傳 `session_id`；agy `--conversation`），追加指示時以 `--resume` / `--conversation` 執行。

- [ ] **花費與結果摘要**
  改用 `--output-format json`（或 stream-json）擷取最終結果、token 數與費用，顯示在卡片與抽屜上。

- [ ] **Runner 在線狀態**
  側邊欄目前只顯示 API 是否連線。
  做法：Runner 每次輪詢順便回報 `agent_id`，後端記錄最後出現時間；側邊欄顯示「@agy Runner：在線 / 離線、正在執行 #xxxx」。

- [ ] **依賴關係圖（DAG）**
  Claude 一次建立一串有依賴的任務時，用圖一眼看出卡在哪裡。

## 已知限制（暫不處理）

- 任務列表只回傳執行中 / 排隊中的全部任務與最近 200 筆已結束任務，尚無完整分頁或封存。
- worktree 的 diff 統計不含尚未加入 git 的新檔案（新檔案仍會列在「未 commit 的變更」）。
- 合併一律 merge 進 repo 目前 checkout 的分支；repo 有未 commit 的衝突變更時合併會失敗並自動 abort。
- 日誌沒有上限，長時間執行的 Agent 會讓單一任務的日誌很大。
