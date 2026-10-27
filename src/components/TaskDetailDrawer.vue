<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import type { Task, TaskLog } from "../types";
import { STATUS_META, agentMeta, relativeTime } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  task: Task | null;
  logs: TaskLog[];
}>();

const emit = defineEmits<{
  (e: "close"): void;
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
      <button
        @click="emit('close')"
        title="關閉 (Esc)"
        class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink"
      >
        <Icon name="x" />
      </button>
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
