# 🤖 AGENTS.md - Agent 開發協作規範

本文件規範所有參與此專案之 AI Agent（包含 Claude、agy、Antigravity 等）的行為準則與協作規則。

---

## 📌 核心鐵律 (Golden Rules)

### 1. 文檔與規格同步原則（最重要！）
> **「有做功能或架構異動，必須即時更新規格或文件！」**
- 凡是**新增 API 端點、調整資料庫 Schema、新增前端功能、變更執行流程**，均必須同步更新至對應規格文件：
  - 架構與整體流程：[`doc/plan.md`](doc/plan.md)
  - 任務追蹤與交接：[`doc/todo.md`](doc/todo.md)、[`doc/handoff.md`](doc/handoff.md)
  - 使用者面向說明：[`README.md`](README.md)
- **嚴禁**只修改程式碼而讓文件規格落後或不一致。

### 2. 嚴格禁止未經指示自行 Commit (No Unsolicited Commits)
- 在使用者給出明確提交指令（如「請 commit」、「幫我 commit」）之前，**絕對不可以（NEVER）** 執行任何 `git commit`。
- 所有變更一律保留在 Working Tree / Staging 狀態。

### 3. 最簡可行原則 (Ponytail / YAGNI)
- 優先使用標準庫，避免不必要的外部依賴。
- 拒絕過度設計與非必要的複雜抽象。

---

## 🛠️ 專案技術棧速查
- **前端**：Tauri v2 + Vue 3 (TypeScript) + Tailwind CSS v4 + `@tauri-apps/plugin-dialog`
- **後端**：Rust + Axum (REST API & SSE on `:18833`) + SQLite (WAL mode, sqlx)
- **外部整合**：`agent-board mcp` (原生 MCP Stdio 服務)、`agent-board runner` (原生背景輪詢 Worker)
