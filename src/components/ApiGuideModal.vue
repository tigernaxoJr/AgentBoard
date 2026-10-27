<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { apiConfig, type ApiConfig } from "../composables/useTasks";

const emit = defineEmits<{
  (e: "close"): void;
}>();

const copiedIndex = ref<number | null>(null);
const expandedEndpoint = ref<string | null>(null);
const installingAgent = ref<string | null>(null);
const installMessage = ref<{ type: "success" | "error"; text: string } | null>(null);

const currentConfig = ref<ApiConfig>({
  base_url: "http://127.0.0.1:18833",
  token: "",
  project_dir: "",
  token_file_path: "",
});

onMounted(async () => {
  currentConfig.value = await apiConfig;
});

const handleInstallMcp = async (agent: "claude" | "agy") => {
  installingAgent.value = agent;
  installMessage.value = null;
  try {
    const res = await invoke<string>("install_mcp_server", { agent });
    installMessage.value = {
      type: "success",
      text: `已成功將 MCP Server 註冊至 ${agent}！\n${res}`,
    };
  } catch (err: any) {
    installMessage.value = {
      type: "error",
      text: `註冊至 ${agent} 失敗：${err?.message || err}`,
    };
  } finally {
    installingAgent.value = null;
  }
};

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

const aiPromptSnippet = computed(() => {
  const tokenPart = currentConfig.value.token
    ? `（目前 API Token: ${currentConfig.value.token}）`
    : "";
  return `請先執行 curl -s ${currentConfig.value.base_url}/api/spec 閱讀看板操作規格${tokenPart}，之後依照規格將任務發派到看板並追蹤完成。`;
});

const mcpCommand = computed(() => {
  if (currentConfig.value.exe_path) {
    return `claude mcp add agent-board "${currentConfig.value.exe_path}" mcp`;
  }
  return `claude mcp add agent-board agent-board mcp`;
});

interface EndpointDef {
  method: string;
  path: string;
  desc: string;
  curl: string;
}

const endpoints = computed<EndpointDef[]>(() => {
  const base = currentConfig.value.base_url;
  const auth = tokenAuthHeader.value;

  return [
    {
      method: "POST",
      path: "/api/tasks",
      desc: "建立新任務（支援前置任務 DAG）",
      curl: `curl -X POST ${base}/api/tasks \\
  ${auth} \\
  -H "Content-Type: application/json" \\
  -d '{
    "title": "重構使用者模組",
    "prompt": "請優化 src/features/tasks 並加上單元測試",
    "creator": "claude",
    "assigned_agent": "agy"
  }'`,
    },
    {
      method: "GET",
      path: "/api/tasks/{id}",
      desc: "查詢任務狀態與結果（status 為 completed 時讀取 result）",
      curl: `curl -s ${base}/api/tasks/<task_id> \\
  ${auth}`,
    },
    {
      method: "GET",
      path: "/api/tasks",
      desc: "列出任務列表（支援關鍵字搜尋 ?q=keyword）",
      curl: `curl -s "${base}/api/tasks?q=重構" \\
  ${auth}`,
    },
    {
      method: "POST",
      path: "/api/tasks/{id}/spawn",
      desc: "由面板立即在本機執行（body：agent_id、model、workdir、isolate…）",
      curl: `curl -X POST ${base}/api/tasks/<task_id>/spawn \\
  ${auth} \\
  -H "Content-Type: application/json" \\
  -d '{"agent_id": "agy", "model": "gemini-2.5-pro"}'`,
    },
    {
      method: "POST",
      path: "/api/tasks/{id}/dispatch",
      desc: "派發給 Runner，下次輪詢優先領取",
      curl: `curl -X POST ${base}/api/tasks/<task_id>/dispatch \\
  ${auth}`,
    },
    {
      method: "POST",
      path: "/api/tasks/{id}/followup",
      desc: "追加指示，建立接續同一工作階段（Session）的新任務",
      curl: `curl -X POST ${base}/api/tasks/<task_id>/followup \\
  ${auth} \\
  -H "Content-Type: application/json" \\
  -d '{"prompt": "請依照上述實作補上端對端測試"}'`,
    },
    {
      method: "POST",
      path: "/api/tasks/{id}/cancel",
      desc: "取消排隊中或執行中的任務",
      curl: `curl -X POST ${base}/api/tasks/<task_id>/cancel \\
  ${auth}`,
    },
    {
      method: "POST",
      path: "/api/tasks/{id}/requeue",
      desc: "失敗的任務重新排隊（連帶失敗的下游一併恢復）",
      curl: `curl -X POST ${base}/api/tasks/<task_id>/requeue \\
  ${auth} \\
  -H "Content-Type: application/json" \\
  -d '{"fresh": true}'`,
    },
    {
      method: "POST",
      path: "/api/tasks/{id}/artifacts",
      desc: "回報成果檔案",
      curl: `curl -X POST ${base}/api/tasks/<task_id>/artifacts \\
  ${auth} \\
  -H "Content-Type: application/json" \\
  -d '{"agent_id": "agy", "file_path": "dist/output.csv", "summary": "產出報表"}'`,
    },
    {
      method: "GET",
      path: "/api/tasks/{id}/worktree",
      desc: "隔離執行的分支狀態（diff 與合併狀態）",
      curl: `curl -s ${base}/api/tasks/<task_id>/worktree \\
  ${auth}`,
    },
    {
      method: "GET",
      path: "/api/runners",
      desc: "各 Agent 的 Runner 在線狀態",
      curl: `curl -s ${base}/api/runners \\
  ${auth}`,
    },
    {
      method: "GET",
      path: "/api/agents",
      desc: "取得已註冊之 Agent 與支援的模型清單",
      curl: `curl -s ${base}/api/agents \\
  ${auth}`,
    },
  ];
});

const toggleEndpoint = (path: string) => {
  expandedEndpoint.value = expandedEndpoint.value === path ? null : path;
};
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
        <!-- 核心 1: /api/spec 一句話讓 AI 讀懂規格 -->
        <div class="bg-indigo-50/50 dark:bg-indigo-950/20 p-4 rounded-xl border border-indigo-200/70 dark:border-indigo-800/60">
          <h4 class="font-bold text-sm text-indigo-900 dark:text-indigo-200 mb-1.5 flex items-center gap-1.5">
            <span>✨ 方式一：一句話讓 AI 讀規格自動操作（免裝工具）</span>
          </h4>
          <p class="text-slate-600 dark:text-slate-400 mb-2 leading-relaxed">
            後端內建免驗證的 <code class="bg-white dark:bg-slate-800 px-1 py-0.5 rounded font-mono text-indigo-600 dark:text-indigo-400">/api/spec</code> 端點。只要複製這段貼給終端 AI（Claude Code、Windsurf、Cursor、agy 等），它就會自動讀懂規格、發工並輪詢收成果：
          </p>
          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(aiPromptSnippet, 0)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-indigo-600 hover:bg-indigo-500 text-white font-sans text-xs transition-colors"
            >
              {{ copiedIndex === 0 ? "✓ 已複製" : "複製貼給 AI" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre-wrap leading-relaxed pr-24">{{ aiPromptSnippet }}</pre>
          </div>
        </div>

        <!-- 核心 2: MCP Server -->
        <div class="bg-slate-50 dark:bg-slate-800/40 p-4 rounded-xl border border-slate-200 dark:border-slate-800">
          <div class="flex items-center justify-between mb-1.5">
            <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 flex items-center gap-1.5">
              <span>🔌 方式二：使用原生 MCP 工具接入 (Claude / agy)</span>
            </h4>
            <!-- 一鍵安裝按鈕組 -->
            <div class="flex items-center gap-1.5">
              <button
                @click="handleInstallMcp('claude')"
                :disabled="installingAgent !== null"
                class="px-2.5 py-1 rounded bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white font-sans text-[11px] transition-colors flex items-center gap-1"
                title="自動執行 claude mcp add agent-board ..."
              >
                <span>{{ installingAgent === 'claude' ? '註冊中...' : '⚡ 一鍵加入 Claude' }}</span>
              </button>
              <button
                @click="handleInstallMcp('agy')"
                :disabled="installingAgent !== null"
                class="px-2.5 py-1 rounded bg-teal-600 hover:bg-teal-500 disabled:opacity-50 text-white font-sans text-[11px] transition-colors flex items-center gap-1"
                title="自動執行 agy mcp add agent-board ..."
              >
                <span>{{ installingAgent === 'agy' ? '註冊中...' : '⚡ 一鍵加入 agy' }}</span>
              </button>
            </div>
          </div>

          <p class="text-slate-500 mb-2 leading-relaxed">
            點擊上方一鍵安裝或手動執行下方指令。註冊後 AI 直接擁有 <code class="font-mono">create_task</code>、<code class="font-mono">list_tasks</code>、<code class="font-mono">spawn_task</code> 等工具：
          </p>

          <!-- 安裝反饋訊息 -->
          <div
            v-if="installMessage"
            class="mb-2 p-2.5 rounded-lg text-[11px] font-mono whitespace-pre-line border"
            :class="installMessage.type === 'success' ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-800 dark:text-emerald-300 border-emerald-200 dark:border-emerald-800' : 'bg-rose-50 dark:bg-rose-950/40 text-rose-800 dark:text-rose-300 border-rose-200 dark:border-rose-800'"
          >
            {{ installMessage.text }}
          </div>

          <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800">
            <button
              @click="copyText(mcpCommand, 1)"
              class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs transition-colors"
            >
              {{ copiedIndex === 1 ? "✓ 已複製" : "複製指令" }}
            </button>
            <pre class="overflow-x-auto whitespace-pre leading-relaxed pr-20">{{ mcpCommand }}</pre>
          </div>
        </div>

        <!-- 折疊清單: API Reference -->
        <div>
          <div class="flex items-center justify-between mb-2">
            <h4 class="font-bold text-sm text-slate-900 dark:text-slate-100 flex items-center gap-1.5">
              <span>📚 API 端點手冊（點擊展開詳細 curl 指令）</span>
            </h4>
            <span class="text-[11px] text-slate-400">共 {{ endpoints.length }} 個端點</span>
          </div>

          <div class="rounded-xl border border-slate-200 dark:border-slate-800 divide-y divide-slate-100 dark:divide-slate-800 overflow-hidden">
            <div
              v-for="(ep, idx) in endpoints"
              :key="ep.path"
              class="transition-colors"
            >
              <button
                @click="toggleEndpoint(ep.path)"
                class="w-full flex items-center justify-between px-3.5 py-2.5 text-left hover:bg-slate-50 dark:hover:bg-slate-800/60 transition-colors"
              >
                <div class="flex items-center gap-2.5 min-w-0">
                  <span
                    class="w-12 shrink-0 font-mono text-[10px] font-semibold text-center px-1 py-0.5 rounded"
                    :class="ep.method === 'POST' ? 'bg-indigo-50 text-indigo-600 dark:bg-indigo-950/40 dark:text-indigo-400' : 'bg-emerald-50 text-emerald-600 dark:bg-emerald-950/40 dark:text-emerald-400'"
                  >
                    {{ ep.method }}
                  </span>
                  <code class="shrink-0 font-mono text-[11px] text-slate-900 dark:text-slate-100">{{ ep.path }}</code>
                  <span class="text-slate-500 truncate text-[11px]">{{ ep.desc }}</span>
                </div>
                <span class="text-slate-400 text-xs shrink-0 ml-2">
                  {{ expandedEndpoint === ep.path ? "▲" : "▼" }}
                </span>
              </button>

              <!-- 折疊展開的詳細 curl -->
              <div
                v-if="expandedEndpoint === ep.path"
                class="px-3.5 pb-3.5 pt-1 bg-slate-50/50 dark:bg-slate-950/30"
              >
                <div class="relative bg-slate-950 text-slate-200 rounded-xl p-3 font-mono text-[11px] border border-slate-800 mt-1">
                  <button
                    @click="copyText(ep.curl, 100 + idx)"
                    class="absolute top-2.5 right-2.5 px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-slate-300 text-[10px] transition-colors"
                  >
                    {{ copiedIndex === 100 + idx ? "✓ 已複製" : "複製 curl" }}
                  </button>
                  <pre class="overflow-x-auto whitespace-pre leading-relaxed pr-20">{{ ep.curl }}</pre>
                </div>
              </div>
            </div>
          </div>
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
