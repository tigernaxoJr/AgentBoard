<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Artifact, Task, TaskLog, WorktreeStatus } from "../types";
import { STATUS_META, agentMeta, relativeTime } from "../ui";
import Icon from "./Icon.vue";
import WorktreePanel from "./WorktreePanel.vue";

const props = defineProps<{
  task: Task | null;
  logs: TaskLog[];
  artifacts: Artifact[];
  loadWorktree: (taskId: string) => Promise<WorktreeStatus>;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "cancel"): void;
  (e: "requeue"): void;
  (e: "merge-worktree"): void;
  (e: "discard-worktree"): void;
}>();

const logBox = ref<HTMLElement | null>(null);
const copied = ref(false);

// 新日誌進來時自動捲到底
watch(
  () => props.logs.length,
  async () => {
    await nextTick();
    logBox.value?.scrollTo({ top: logBox.value.scrollHeight });
  }
);

const copyPrompt = async () => {
  if (!props.task) return;
  await navigator.clipboard.writeText(props.task.prompt);
  copied.value = true;
  setTimeout(() => (copied.value = false), 2000);
};

// 成果路徑若是相對路徑，以任務的工作目錄為基準
const resolvePath = (path: string) =>
  path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path) || !props.task?.workdir ? path : `${props.task.workdir}/${path}`;

// 只在檔案管理器中顯示，不直接開啟：路徑由 Agent 回報，直接開啟可能執行到腳本
const reveal = async (path: string) => {
  try {
    await revealItemInDir(resolvePath(path));
  } catch (err) {
    alert(`無法顯示 ${path}：${err}`);
  }
};

const logLevelClass = (level: string) =>
  ({ INFO: "text-sky-400", WARN: "text-amber-400", ERROR: "text-rose-400" })[level] ?? "text-slate-400";
</script>

<template>
  <aside
    v-if="task"
    class="w-[420px] shrink-0 border-l border-line bg-surface flex flex-col h-full"
  >
    <!-- Header -->
    <div class="h-14 shrink-0 px-5 border-b border-line flex items-center justify-between">
      <div class="flex items-center gap-2.5">
        <span :class="['inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium ring-1 ring-inset', STATUS_META[task.status].soft]">
          <span :class="['h-1.5 w-1.5 rounded-full', STATUS_META[task.status].dot, task.status === 'running' && 'animate-pulse']"></span>
          {{ STATUS_META[task.status].label }}
        </span>
        <span class="font-mono text-xs text-ink-3 select-text">#{{ task.id.slice(0, 8) }}</span>
      </div>
      <div class="flex items-center gap-1">
        <button
          v-if="task.status === 'failed'"
          @click="emit('requeue')"
          title="重新排隊（領取次數歸零，連帶失敗的下游任務一併恢復）"
          class="flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-accent hover:bg-accent-soft"
        >
          <Icon name="refresh" :size="12" />
          重新排隊
        </button>
        <button
          v-if="task.status === 'queued' || task.status === 'running'"
          @click="emit('cancel')"
          title="取消任務（執行中的 Agent 會被中止）"
          class="h-7 rounded-md px-2 text-xs font-medium text-rose-600 hover:bg-rose-500/10 dark:text-rose-400"
        >
          取消任務
        </button>
        <button
          @click="emit('close')"
          title="關閉 (Esc)"
          class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink"
        >
          <Icon name="x" />
        </button>
      </div>
    </div>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto px-5 py-5 flex flex-col gap-6 text-xs">
      <h2 class="text-base font-semibold leading-snug text-ink select-text">{{ task.title }}</h2>

      <dl class="grid grid-cols-[88px_1fr] gap-y-2.5 text-xs">
        <dt class="text-ink-3">指定 Agent</dt>
        <dd>
          <span :class="['inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium ring-1 ring-inset', agentMeta(task.assigned_agent).chip]">
            <span :class="['h-1.5 w-1.5 rounded-full', agentMeta(task.assigned_agent).dot]"></span>
            {{ task.assigned_agent ? `@${task.assigned_agent}` : "任意 Agent" }}
          </span>
        </dd>
        <dt class="text-ink-3">模型</dt>
        <dd class="font-mono text-ink-2">{{ task.model ?? "預設" }}</dd>
        <template v-if="task.workdir">
          <dt class="text-ink-3">工作目錄</dt>
          <dd class="flex items-start gap-1.5">
            <span class="font-mono text-ink-2 break-all select-text">{{ task.workdir }}</span>
            <button @click="reveal(task.workdir)" title="在檔案管理器中顯示" class="shrink-0 text-ink-3 hover:text-ink">
              <Icon name="folder" :size="12" />
            </button>
          </dd>
        </template>
        <template v-if="task.isolate">
          <dt class="text-ink-3">隔離</dt>
          <dd class="text-ink-2">git worktree</dd>
        </template>
        <template v-if="task.auto_approve">
          <dt class="text-ink-3">權限</dt>
          <dd class="font-medium text-rose-600 dark:text-rose-400">完整權限</dd>
        </template>
        <dt class="text-ink-3">建立者</dt>
        <dd class="text-ink">{{ task.creator }}</dd>
        <dt class="text-ink-3">建立時間</dt>
        <dd class="text-ink-2">{{ new Date(task.created_at).toLocaleString() }}</dd>
        <dt class="text-ink-3">領取次數</dt>
        <dd class="text-ink-2">{{ task.attempts }} / 3</dd>
        <dt class="text-ink-3">心跳回報</dt>
        <dd class="flex items-center gap-1.5 text-ink-2">
          <Icon v-if="task.last_heartbeat_at" name="heart" :size="12" class="text-sky-500" />
          {{ task.last_heartbeat_at ? relativeTime(task.last_heartbeat_at) : "無" }}
        </dd>
        <template v-if="task.depends_on_task_id">
          <dt class="text-ink-3">前置任務</dt>
          <dd class="flex items-center gap-1 font-mono text-amber-600 dark:text-amber-400 select-text">
            <Icon name="link" :size="12" />
            #{{ task.depends_on_task_id.slice(0, 8) }}
          </dd>
        </template>
      </dl>

      <section>
        <div class="mb-2 flex items-center justify-between">
          <h4 class="text-[11px] font-semibold uppercase tracking-wider text-ink-3">Prompt</h4>
          <button
            @click="copyPrompt"
            class="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] text-ink-3 hover:bg-raised hover:text-ink"
          >
            <Icon :name="copied ? 'check' : 'copy'" :size="12" />
            {{ copied ? "已複製" : "複製" }}
          </button>
        </div>
        <div class="max-h-56 overflow-y-auto rounded-lg border border-line bg-raised/60 p-3 font-mono text-[12px] leading-relaxed text-ink whitespace-pre-wrap select-text">{{ task.prompt }}</div>
      </section>

      <WorktreePanel
        v-if="task.branch"
        :task="task"
        :load="loadWorktree"
        @merge="emit('merge-worktree')"
        @discard="emit('discard-worktree')"
      />

      <!-- Artifacts -->
      <section v-if="artifacts.length">
        <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-ink-3">成果 · {{ artifacts.length }}</h4>
        <ul class="flex flex-col gap-1.5">
          <li v-for="a in artifacts" :key="a.id" class="group flex items-start gap-2 rounded-lg border border-line px-3 py-2">
            <Icon name="file" :size="13" class="mt-0.5 text-ink-3" />
            <div class="min-w-0 flex-1">
              <div class="font-mono text-[12px] text-ink break-all select-text">{{ a.file_path }}</div>
              <div v-if="a.summary" class="mt-0.5 text-ink-2 select-text">{{ a.summary }}</div>
              <div class="mt-0.5 text-[11px] text-ink-3">@{{ a.agent_id }} · {{ relativeTime(a.created_at) }}</div>
            </div>
            <button @click="reveal(a.file_path)" title="在檔案管理器中顯示" class="shrink-0 rounded-md p-1 text-ink-3 hover:bg-raised hover:text-ink">
              <Icon name="folder" :size="13" />
            </button>
          </li>
        </ul>
      </section>

      <!-- Execution Logs -->
      <section class="flex flex-1 flex-col min-h-56">
        <div class="mb-2 flex items-center justify-between">
          <h4 class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-ink-3">
            <Icon name="terminal" :size="12" />
            Live Logs
          </h4>
          <span class="font-mono text-[11px] text-ink-3">{{ logs.length }} 則</span>
        </div>
        <div
          ref="logBox"
          class="flex-1 min-h-0 max-h-80 overflow-y-auto rounded-lg border border-line bg-term p-3 font-mono text-[11px] leading-relaxed text-slate-300 select-text"
        >
          <div v-if="logs.length === 0" class="py-6 text-center text-slate-600">
            <span class="animate-pulse">▍</span> 等待 Agent 輸出…
          </div>
          <div v-for="log in logs" :key="log.id" class="flex gap-2">
            <span class="shrink-0 text-slate-600">{{ new Date(log.created_at).toLocaleTimeString([], { hour12: false }) }}</span>
            <span :class="['shrink-0 w-10 font-semibold', logLevelClass(log.level)]">{{ log.level }}</span>
            <span class="min-w-0 break-words"><span class="text-teal-400">{{ log.agent_id }}</span> {{ log.message }}</span>
          </div>
        </div>
      </section>
    </div>
  </aside>
</template>
