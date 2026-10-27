<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { Task } from "../types";
import { agentMeta, formatCost, relativeTime } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  task: Task;
  isSelected: boolean;
}>();

const emit = defineEmits<{
  (e: "select", task: Task): void;
  (e: "run"): void;
}>();

const copied = ref(false);
const nowSec = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;

onMounted(() => {
  if (props.task.status === "running") {
    timer = setInterval(() => {
      nowSec.value = Date.now();
    }, 1000);
  }
});

onUnmounted(() => {
  if (timer) clearInterval(timer);
});

// 即時累積執行時間碼表（格式：mm:ss 或 hh:mm:ss）
const runningDuration = computed(() => {
  if (props.task.status !== "running") return null;
  const start = new Date(props.task.updated_at).getTime();
  const diffSec = Math.max(0, Math.floor((nowSec.value - start) / 1000));
  const m = Math.floor(diffSec / 60);
  const s = diffSec % 60;
  if (m < 60) {
    return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  const h = Math.floor(m / 60);
  const remM = m % 60;
  return `${h}:${String(remM).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
});

const copyPrompt = async () => {
  try {
    await navigator.clipboard.writeText(props.task.prompt);
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 2000);
  } catch (err) {
    console.error("Failed to copy", err);
  }
};
</script>

<template>
  <article
    @click="emit('select', task)"
    :class="[
      'group relative overflow-hidden rounded-lg border bg-surface p-3.5 cursor-pointer transition-[border-color,box-shadow] duration-150 flex flex-col gap-2',
      isSelected
        ? 'border-accent shadow-[0_0_0_3px_var(--accent-soft)]'
        : 'border-line hover:border-line-strong hover:shadow-sm'
    ]"
  >
    <!-- 執行中：頂部流動進度條 -->
    <div v-if="task.status === 'running'" class="absolute inset-x-0 top-0 h-0.5 overflow-hidden bg-sky-500/15">
      <div class="h-full w-1/3 bg-sky-500 animate-[indeterminate_1.6s_ease-in-out_infinite]"></div>
    </div>

    <div class="flex items-center justify-between gap-2 text-[11px]">
      <span class="flex items-center gap-1.5">
        <span class="font-mono text-ink-3">#{{ task.id.slice(0, 8) }}</span>
        <span v-if="task.status === 'queued' && task.priority > 0" class="rounded bg-accent-soft px-1 font-medium text-accent" title="已派發，Agent 下次輪詢會優先領取">優先</span>
        <span v-if="task.attempts > 1" class="text-amber-600 dark:text-amber-400" title="逾時重試次數">重試 {{ task.attempts - 1 }}</span>
      </span>
      <span :class="['inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-medium ring-1 ring-inset', agentMeta(task.assigned_agent).chip]">
        <span :class="['h-1.5 w-1.5 rounded-full', agentMeta(task.assigned_agent).dot]"></span>
        {{ task.assigned_agent ? `@${task.assigned_agent}` : "任意" }}
      </span>
    </div>

    <h3 class="text-[13px] font-semibold leading-snug text-ink line-clamp-2">{{ task.title }}</h3>
    <p class="text-xs leading-relaxed text-ink-2 line-clamp-2">{{ task.prompt }}</p>

    <footer class="mt-1 flex items-center justify-between text-[11px] text-ink-3">
      <div class="flex items-center gap-2 min-w-0">
        <span class="flex h-4 w-4 items-center justify-center rounded-full bg-raised text-[9px] font-bold uppercase text-ink-2">
          {{ task.creator.slice(0, 1) }}
        </span>
        <span class="truncate">{{ task.creator }} · {{ relativeTime(task.status === 'running' ? task.last_heartbeat_at : task.updated_at) }}</span>
        <span v-if="runningDuration" class="inline-flex items-center gap-1 font-mono text-[10px] text-sky-500 font-semibold" title="即時執行累計耗時">
          <span class="inline-block h-1.5 w-1.5 rounded-full bg-sky-500 animate-ping"></span>
          {{ runningDuration }}
        </span>
        <span v-if="task.dependency_ids.length" class="text-amber-500" :title="`依賴 ${task.dependency_ids.map((d) => '#' + d.slice(0, 8)).join(', ')}`">
          <Icon name="link" :size="12" />
        </span>
        <span v-if="formatCost(task.cost_usd)" class="font-mono" title="這次執行的花費">{{ formatCost(task.cost_usd) }}</span>
      </div>

      <div class="flex items-center gap-1" @click.stop>
        <button
          @click="copyPrompt"
          :title="copied ? '已複製' : '複製完整提示詞'"
          :class="[
            'flex h-6 w-6 items-center justify-center rounded-md transition-all hover:bg-raised hover:text-ink',
            copied ? 'text-emerald-500 opacity-100' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'
          ]"
        >
          <Icon :name="copied ? 'check' : 'copy'" :size="13" />
        </button>
        <button
          v-if="task.status === 'queued'"
          @click="emit('run')"
          title="選擇 Agent 與模型後立即執行，或派發給 Runner"
          class="flex h-6 items-center gap-1 rounded-md bg-accent-soft px-2 font-medium text-accent transition-colors hover:bg-accent hover:text-accent-ink"
        >
          <Icon name="play" :size="10" />
          執行
        </button>
      </div>
    </footer>
  </article>
</template>
