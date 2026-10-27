<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { apiConfig, type ApiConfig } from "../composables/useTasks";

const emit = defineEmits<{
  (e: "close"): void;
}>();

const DEP_RESULT = "{{dep.result}}";
const copiedIndex = ref<number | null>(null);
const currentConfig = ref<ApiConfig>({
  base_url: "http://127.0.0.1:18833",
  token: "",
  project_dir: "",
  token_file_path: "",
});

onMounted(async () => {
  currentConfig.value = await apiConfig;
});

const copyText = async (text: string, index: number) => {
  try {
    await navigator.clipboard.writeText(text);
    copiedIndex.value = index;
    setTimeout(() => {
      copiedIndex.value = null;
    }, 2000);
  } catch (err) {
    console.error("Failed to copy", err);
  }
};

const tokenAuthHeader = computed(() => {
  if (currentConfig.value.token) {
    return `-H "Authorization: Bearer ${currentConfig.value.token}"`;
  }
  if (currentConfig.value.token_file_path) {
    return `-H "Authorization: Bearer $(cat '${currentConfig.value.token_file_path}')"`;
  }
  return `-H "Authorization: Bearer $(cat ~/.local/share/com.naxo.agent-board/api_token)"`;
});

const curlCreateTask = computed(() => `curl -X POST ${currentConfig.value.base_url}/api/tasks \\
  ${tokenAuthHeader.value} \\
  -H "Content-Type: application/json" \\
  -d '{
    "title": "重構使用者模組",
    "prompt": "請優化 src/features/tasks 並加上單元測試",
    "creator": "claude",
    "assigned_agent": "agy"
  }'`);

const curlCreateWithDep = computed(() => `curl -X POST ${currentConfig.value.base_url}/api/tasks \\
  ${tokenAuthHeader.value} \\
  -H "Content-Type: application/json" \\
  -d '{
    "title": "產出報表",
    "prompt": "讀取前置產生的 CSV 檔產生圖表",
    "creator": "claude",
    "assigned_agent": "agy",
    "dependency_ids": ["<前置任務ID>"]
  }'`);

// 常用端點速查（都需要 Authorization: Bearer <token>）
const endpoints: [string, string, string][] = [
  ["GET", "/api/spec", "看板完整規格說明書（免 Token，供 AI 直接閱讀學會操作）"],
  ["POST", "/api/tasks/{id}/spawn", "由面板立即在本機執行（body：agent_id、model、workdir、isolate…）"],
  ["POST", "/api/tasks/{id}/dispatch", "派發給 Runner，下次輪詢優先領取"],
  ["POST", "/api/tasks/{id}/followup", "追加指示（body：prompt），建立接續同一工作階段的新任務"],
  ["POST", "/api/tasks/{id}/cancel", "取消排隊中或執行中的任務"],
  ["POST", "/api/tasks/{id}/requeue", "失敗的任務重新排隊（連帶失敗的下游一併恢復）；body {\"fresh\": true} 捨棄工作階段從頭執行"],
  ["POST", "/api/tasks/{id}/artifacts", "回報成果檔案（body：agent_id、file_path、summary）"],
  ["GET", "/api/tasks/{id}/worktree", "隔離執行的分支狀態；merge / discard 用 POST …/worktree/merge|discard"],
  ["GET", "/api/runners", "各 Agent 的 Runner 在線狀態"],
];

const aiPromptSnippet = computed(() => {
  const tokenPart = currentConfig.value.token
    ? `（目前 API Token: ${currentConfig.value.token}）`
    : "";
  return `請先執行 curl -s ${currentConfig.value.base_url}/api/spec 閱讀看板操作規格${tokenPart}，之後依照規格將任務發派到看板並追蹤完成。`;
});

const mcpScriptPath = computed(() => {
  if (currentConfig.value.project_dir) {
    // 轉為正斜線風格路徑以相容 CLI / JSON 設定
    const normalized = currentConfig.value.project_dir.replace(/\\/g, "/");
    return `${normalized}/scripts/mcp_server.py`;
  }
  return "/path/to/agent_board/scripts/mcp_server.py";
});

const mcpSnippet = computed(() => `claude mcp add agent-board python3 "${mcpScriptPath.value}"`);

const systemdSnippet = `mkdir -p ~/.config/systemd/user
cp scripts/agent-board-runner.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now agent-board-runner.service`;
</script>

<template>
  <div class="fixed inset-0 bg-slate-900/60 backdrop-blur-xs flex items-center justify-center p-4 z-50">
    <div class="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl w-full max-w-2xl shadow-2xl overflow-hidden flex flex-col max-h-[85vh]">
      <!-- Header -->
      <div class="p-5 border-b border-slate-100 dark:border-slate-800 flex items-center justify-between shrink-0">
        <div class="flex items-center gap-2">
          <span class="text-lg">📖</span>
          <h3 class="text-base font-semibold text-slate-900 dark:text-slate-100">
            Agent 派工與 API 接入指南
          </h3>
        </div>
        <button
          @click="emit('close')"
          class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-lg leading-none"
        >
          &times;
        </button>
      </div>

      <!-- Content -->
      <div class="p-6 overflow-y-auto flex flex-col gap-6 text-xs text-slate-700 dark:text-slate-300">
        <!-- 說明 1: 最強推薦，一句話讓 AI 讀規格 -->
        <div class="bg-indigo-50/50 dark:bg-indigo-950/20 p-4 rounded-xl border border-indigo-200/70 dark:border-indigo-800/60">
          <h4 class="font-bold text-sm text-indigo-900 dark:text-indigo-200 mb-1.5 flex items-center gap-1.5">
            <span>✨ 最簡推薦：一句話教 AI 自動操作看板</span>
          </h4>
          <p class="text-slate-600 dark:text-slate-400 mb-2 leading-relaxed">
            後端內建免驗證的 <code class="bg-white dark:bg-slate-800 px-1 py-0.5 rounded font-mono text-indigo-600 dark:text-indigo-400">/api/spec</code> 說明書。你只需把這句話複製貼給 Claude Code、Windsurf 或任何終端 AI，它就會自動讀懂規格、發工並等待結果：
          </p>
          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(aiPromptSnippet, 0)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-indigo-600 hover:bg-indigo-500 text-white font-sans transition-colors"
            >
              {{ copiedIndex === 0 ? "✓ 已複製" : "複製貼給 AI" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre-wrap leading-relaxed pr-24">{{ aiPromptSnippet }}</pre>
          </div>
        </div>

        <!-- 說明 2: 手動丟任務 -->
        <div>
          <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 mb-1.5 flex items-center gap-1.5">
            <span>1. 手動用 curl 派工與等待結果</span>
          </h4>
          <p class="text-slate-500 mb-2 leading-relaxed">
            發送 POST 建立任務，並定期 GET 查詢結果是否完成（<code class="font-mono">status == "completed"</code>）：
          </p>
          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(curlCreateTask, 1)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-300 transition-colors"
            >
              {{ copiedIndex === 1 ? "✓ 已複製" : "複製指令" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre leading-relaxed">{{ curlCreateTask }}</pre>
          </div>
        </div>

        <!-- 說明 3: 連鎖相依 (DAG) -->
        <div>
          <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 mb-1.5 flex items-center gap-1.5">
            <span>2. 設定連鎖相依任務 (DAG，自動代入前置成果)</span>
          </h4>
          <p class="text-slate-500 mb-2 leading-relaxed">
            帶入 <code class="bg-slate-100 dark:bg-slate-800 px-1 py-0.5 rounded font-mono">dependency_ids</code>，並在 prompt 內寫 <code class="bg-slate-100 dark:bg-slate-800 px-1 py-0.5 rounded font-mono">{{ DEP_RESULT }}</code>。看板會在前置完成時自動替換成果並喚醒任務：
          </p>
          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(curlCreateWithDep, 2)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-300 transition-colors"
            >
              {{ copiedIndex === 2 ? "✓ 已複製" : "複製指令" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre leading-relaxed">{{ curlCreateWithDep }}</pre>
          </div>
        </div>

        <!-- 說明 4: MCP Server -->
        <div>
          <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 mb-1.5 flex items-center gap-1.5">
            <span>3. 使用原生 MCP 工具 (免打 curl)</span>
          </h4>
          <p class="text-slate-500 mb-2 leading-relaxed">
            若習慣以標準 MCP 工具讓 AI 直接呼叫，直接將隨附的 <code class="bg-slate-100 dark:bg-slate-800 px-1 py-0.5 rounded font-mono">scripts/mcp_server.py</code> 加入 Claude：
          </p>
          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(mcpSnippet, 4)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-300 transition-colors"
            >
              {{ copiedIndex === 4 ? "✓ 已複製" : "複製指令" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre leading-relaxed">{{ mcpSnippet }}</pre>
          </div>
        </div>

        <!-- 說明 5: Systemd -->
        <div>
          <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 mb-1.5 flex items-center gap-1.5">
            <span>4. 開機背景待命 Runner (0 Token 常駐)</span>
          </h4>
          <p class="text-slate-500 mb-2 leading-relaxed">
            將專案隨附的 Runner 註冊為 Linux 系統服務，有任務才喚醒 agy：
          </p>
          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(systemdSnippet, 3)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-300 transition-colors"
            >
              {{ copiedIndex === 3 ? "✓ 已複製" : "複製指令" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre leading-relaxed">{{ systemdSnippet }}</pre>
          </div>
        </div>

        <!-- 說明 6: API 速查 -->
        <div>
          <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 mb-1.5">5. 常用 API 速查</h4>
          <div class="rounded-xl border border-slate-200 dark:border-slate-800 divide-y divide-slate-100 dark:divide-slate-800">
            <div v-for="[method, path, desc] in endpoints" :key="path" class="flex items-baseline gap-2 px-3 py-2">
              <span class="w-9 shrink-0 font-mono text-[10px] font-semibold text-indigo-600 dark:text-indigo-400">{{ method }}</span>
              <code class="shrink-0 font-mono text-[11px] text-slate-800 dark:text-slate-200">{{ path }}</code>
              <span class="text-slate-500">{{ desc }}</span>
            </div>
          </div>
        </div>

        <!-- 說明 5 -->
        <div class="bg-slate-50 dark:bg-slate-800/60 p-3.5 rounded-xl border border-slate-200 dark:border-slate-800">
          <h5 class="font-bold text-slate-800 dark:text-slate-200 mb-1">💡 快捷提示：複製 Prompt 貼給純網頁 Agent</h5>
          <p class="text-slate-500 leading-relaxed">
            如果遇到無法直接發送 HTTP 請求的 Agent（如純瀏覽器版 ChatGPT），您可以直接點擊卡片底部的 <span class="font-medium text-slate-700 dark:text-slate-300">📋 Prompt</span> 複製指令手動貼入。
          </p>
        </div>
      </div>

      <!-- Footer -->
      <div class="p-4 border-t border-slate-100 dark:border-slate-800 flex justify-end shrink-0">
        <button
          @click="emit('close')"
          class="px-4 py-2 rounded-lg bg-indigo-600 hover:bg-indigo-700 text-white font-medium text-xs shadow-sm transition-colors"
        >
          我知道了
        </button>
      </div>
    </div>
  </div>
</template>
