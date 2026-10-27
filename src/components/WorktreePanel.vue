<script setup lang="ts">
import { ref, watch } from "vue";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Task, WorktreeStatus } from "../types";
import Icon from "./Icon.vue";

const props = defineProps<{
  task: Task;
  load: (taskId: string) => Promise<WorktreeStatus>;
  loadDiff?: (taskId: string, path?: string) => Promise<string>;
}>();

const emit = defineEmits<{
  (e: "merge"): void;
  (e: "discard"): void;
}>();

const status = ref<WorktreeStatus | null>(null);
const error = ref<string | null>(null);
const loading = ref(false);

const fullDiff = ref<string | null>(null);
const showFullDiff = ref(false);
const loadingDiff = ref(false);

const toggleDiff = async () => {
  if (showFullDiff.value) {
    showFullDiff.value = false;
    return;
  }
  if (!props.loadDiff) return;
  loadingDiff.value = true;
  try {
    fullDiff.value = await props.loadDiff(props.task.id);
    showFullDiff.value = true;
  } catch (err: any) {
    alert("讀取 diff 失敗: " + err.message);
  } finally {
    loadingDiff.value = false;
  }
};

const refresh = async () => {
  loading.value = true;
  try {
    status.value = await props.load(props.task.id);
    error.value = null;
  } catch (err: any) {
    error.value = err.message;
  } finally {
    loading.value = false;
  }
};

// 任務狀態或更新時間改變（例如 Agent 跑完）時重新讀取
watch(() => [props.task.id, props.task.status, props.task.updated_at], refresh, { immediate: true });

const reveal = () => revealItemInDir(props.task.worktree_path!).catch((err) => alert(`無法顯示：${err}`));
</script>

<template>
  <section>
    <div class="mb-2 flex items-center justify-between">
      <h4 class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-ink-3">
        <Icon name="branch" :size="12" />
        Git Worktree
      </h4>
      <button @click="refresh" title="重新整理" class="rounded-md p-1 text-ink-3 hover:bg-raised hover:text-ink">
        <Icon name="refresh" :size="12" :class="loading && 'animate-spin'" />
      </button>
    </div>

    <div class="rounded-lg border border-line">
      <div class="flex items-center gap-2 border-b border-line px-3 py-2">
        <span class="font-mono text-[12px] text-ink select-text">{{ task.branch }}</span>
        <span class="font-mono text-[11px] text-ink-3">← {{ task.base_commit?.slice(0, 7) }}</span>
        <button @click="reveal" title="在檔案管理器中顯示 worktree" class="ml-auto text-ink-3 hover:text-ink">
          <Icon name="folder" :size="13" />
        </button>
      </div>

      <div class="flex flex-col gap-2.5 px-3 py-2.5">
        <p v-if="error" class="text-rose-600 dark:text-rose-400">{{ error }}</p>
        <p v-else-if="status && !status.exists" class="text-amber-600 dark:text-amber-400">worktree 目錄已不存在，可捨棄以清除分支。</p>
        <template v-else-if="status">
          <p v-if="!status.commits.length && !status.changes.length" class="text-ink-3">尚無任何變更</p>
          <div v-if="status.commits.length">
            <div class="mb-1 text-[11px] text-ink-3">Commits · {{ status.commits.length }}</div>
            <div v-for="c in status.commits" :key="c" class="truncate font-mono text-[11px] text-ink-2 select-text">{{ c }}</div>
          </div>
          <div v-if="status.changes.length">
            <div class="mb-1 text-[11px] text-ink-3">未 commit 的變更 · {{ status.changes.length }}</div>
            <div v-for="c in status.changes" :key="c" class="truncate font-mono text-[11px] text-ink-2 select-text whitespace-pre">{{ c }}</div>
          </div>
          <div v-if="status.diff_stat" class="flex flex-col gap-1.5">
            <div class="flex items-center justify-between">
              <span class="text-[11px] text-ink-3">變更統計</span>
              <button
                v-if="loadDiff"
                type="button"
                @click="toggleDiff"
                class="font-mono text-[11px] text-accent hover:underline flex items-center gap-1"
              >
                <span>{{ showFullDiff ? "隱藏完整 Diff" : "檢視完整 Diff" }}</span>
                <span v-if="loadingDiff" class="animate-spin text-[10px]">⧗</span>
              </button>
            </div>
            <pre class="max-h-28 overflow-auto rounded-md bg-raised/60 p-2 font-mono text-[11px] leading-relaxed text-ink-2 select-text">{{ status.diff_stat }}</pre>

            <div v-if="showFullDiff" class="mt-2 rounded-lg border border-line bg-surface overflow-hidden">
              <div class="flex items-center justify-between border-b border-line bg-raised/40 px-2.5 py-1 text-[11px] font-semibold text-ink-3">
                <span>完整 Git Diff</span>
                <button @click="showFullDiff = false" class="hover:text-ink">收起</button>
              </div>
              <pre class="max-h-80 overflow-auto p-2.5 font-mono text-[11px] leading-relaxed text-ink select-text whitespace-pre">{{ fullDiff || '（無具體 diff 內容）' }}</pre>
            </div>
          </div>
        </template>
      </div>

      <div v-if="task.status !== 'running'" class="flex justify-end gap-2 border-t border-line px-3 py-2">
        <button
          @click="emit('discard')"
          class="rounded-md px-2.5 py-1 text-xs font-medium text-rose-600 hover:bg-rose-500/10 dark:text-rose-400"
        >
          捨棄
        </button>
        <button
          @click="emit('merge')"
          :disabled="!status?.exists || (!status.commits.length && !status.changes.length)"
          class="rounded-md bg-accent px-2.5 py-1 text-xs font-semibold text-accent-ink hover:opacity-90 disabled:opacity-40"
        >
          合併回 repo
        </button>
      </div>
    </div>
  </section>
</template>
