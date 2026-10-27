<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useTasks } from "./composables/useTasks";
import TaskCard from "./components/TaskCard.vue";
import CreateTaskModal from "./components/CreateTaskModal.vue";
import TaskDetailDrawer from "./components/TaskDetailDrawer.vue";
import ApiGuideModal from "./components/ApiGuideModal.vue";
import Icon from "./components/Icon.vue";
import { STATUS_META, agentMeta } from "./ui";
import type { CreateTaskPayload, Task, TaskStatus } from "./types";

const {
  tasks,
  queuedTasks,
  runningTasks,
  completedTasks,
  failedTasks,
  selectedTask,
  selectedTaskId,
  activeTaskLogs,
  loading,
  error,
  agentFilter,
  fetchTasks,
  createTask,
  selectTask,
  dispatchTask,
} = useTasks();

const showCreateModal = ref(false);
const showApiGuide = ref(false);
const search = ref("");
const searchInput = ref<HTMLInputElement | null>(null);

const agentFilters = computed(() => [
  { value: "all", label: "全部 Agent", count: tasks.value.length },
  { value: "agy", label: "@agy", count: tasks.value.filter((t) => t.assigned_agent === "agy").length },
  { value: "claude", label: "@claude", count: tasks.value.filter((t) => t.assigned_agent === "claude").length },
  { value: "unassigned", label: "未指定", count: tasks.value.filter((t) => !t.assigned_agent).length },
]);

const matches = (t: Task) => {
  const q = search.value.trim().toLowerCase();
  return !q || [t.title, t.prompt, t.id].some((s) => s.toLowerCase().includes(q));
};

const columns = computed(() =>
  (
    [
      ["queued", queuedTasks.value, "無待領取任務"],
      ["running", runningTasks.value, "目前沒有任務在執行"],
      ["completed", completedTasks.value, "尚無完成紀錄"],
      ["failed", failedTasks.value, "沒有失敗的任務"],
    ] as [TaskStatus, Task[], string][]
  ).map(([status, list, empty]) => ({ status, meta: STATUS_META[status], tasks: list.filter(matches), empty }))
);

const handleCreateTask = async (payload: CreateTaskPayload) => {
  try {
    await createTask(payload);
    showCreateModal.value = false;
  } catch (err: any) {
    alert("建立任務失敗: " + err.message);
  }
};

const handleSelectTask = (task: Task) => {
  selectTask(task.id);
};

const handlePlay = async (task: Task, agent: string) => {
  try {
    await dispatchTask(task.id, agent);
  } catch (err: any) {
    alert("派發失敗: " + err.message);
  }
};

// 鍵盤快捷鍵：N 新增、/ 搜尋、Esc 關閉
const onKeydown = (e: KeyboardEvent) => {
  const typing = e.target instanceof HTMLElement && ["INPUT", "TEXTAREA", "SELECT"].includes(e.target.tagName);
  if (e.key === "Escape") {
    if (showCreateModal.value) showCreateModal.value = false;
    else if (typing) (e.target as HTMLElement).blur();
    else selectedTaskId.value = null;
  } else if (!typing && e.key === "n") {
    e.preventDefault();
    showCreateModal.value = true;
  } else if (!typing && e.key === "/") {
    e.preventDefault();
    searchInput.value?.focus();
  }
};
onMounted(() => window.addEventListener("keydown", onKeydown));
onUnmounted(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <div class="h-screen w-screen flex bg-canvas text-ink select-none overflow-hidden">
    <!-- Sidebar -->
    <aside class="w-56 shrink-0 border-r border-line bg-surface flex flex-col">
      <div class="h-14 px-4 flex items-center gap-2.5 border-b border-line">
        <div class="h-7 w-7 rounded-md bg-accent text-accent-ink flex items-center justify-center">
          <Icon name="layers" :size="15" />
        </div>
        <div class="leading-tight">
          <div class="text-[13px] font-semibold">Agent Board</div>
          <div class="text-[10px] text-ink-3">AOA TaskBoard</div>
        </div>
      </div>

      <nav class="flex-1 overflow-y-auto p-3 flex flex-col gap-6">
        <div>
          <div class="px-2 mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-3">Agents</div>
          <button
            v-for="f in agentFilters"
            :key="f.value"
            @click="agentFilter = f.value"
            :class="[
              'w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors',
              agentFilter === f.value ? 'bg-raised text-ink font-medium' : 'text-ink-2 hover:bg-raised/60 hover:text-ink'
            ]"
          >
            <span v-if="f.value === 'all'" class="h-1.5 w-1.5 rounded-full ring-1 ring-ink-3"></span>
            <span v-else :class="['h-1.5 w-1.5 rounded-full', agentMeta(f.value === 'unassigned' ? null : f.value).dot]"></span>
            <span class="flex-1 text-left">{{ f.label }}</span>
            <span class="font-mono text-[11px] text-ink-3">{{ f.count }}</span>
          </button>
        </div>

        <div>
          <div class="px-2 mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-3">概覽</div>
          <div class="px-2 flex flex-col gap-2.5">
            <div v-for="c in columns" :key="c.status">
              <div class="flex items-center justify-between text-[11px] mb-1">
                <span class="text-ink-2">{{ c.meta.label }}</span>
                <span class="font-mono text-ink-3">{{ c.tasks.length }}</span>
              </div>
              <div class="h-1 rounded-full bg-raised overflow-hidden">
                <div
                  :class="['h-full rounded-full transition-[width] duration-500', c.meta.dot]"
                  :style="{ width: `${tasks.length ? (c.tasks.length / tasks.length) * 100 : 0}%` }"
                ></div>
              </div>
            </div>
          </div>
        </div>
      </nav>

      <div class="border-t border-line px-4 py-3 flex items-center gap-2 text-[11px]">
        <span class="relative flex h-2 w-2">
          <span v-if="!error" class="absolute inline-flex h-full w-full rounded-full bg-emerald-500 opacity-60 animate-ping"></span>
          <span :class="['relative inline-flex h-2 w-2 rounded-full', error ? 'bg-rose-500' : 'bg-emerald-500']"></span>
        </span>
        <span class="text-ink-2">{{ error ? "API 離線" : "API 連線中" }}</span>
        <span class="ml-auto font-mono text-ink-3">:3000</span>
      </div>
    </aside>

    <!-- Main -->
    <div class="flex-1 min-w-0 flex flex-col">
      <header class="h-14 shrink-0 border-b border-line bg-surface px-5 flex items-center gap-3">
        <h1 class="text-sm font-semibold">任務看板</h1>
        <span class="text-xs text-ink-3">{{ tasks.length }} 個任務</span>

        <div class="ml-auto relative w-64">
          <Icon name="search" :size="14" class="absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-3" />
          <input
            ref="searchInput"
            v-model="search"
            placeholder="搜尋標題、Prompt 或 ID"
            class="w-full h-8 rounded-lg border border-line bg-canvas pl-8 pr-8 text-xs text-ink placeholder:text-ink-3 focus:border-accent focus:outline-none focus:ring-3 focus:ring-accent-soft"
          />
          <kbd class="absolute right-2 top-1/2 -translate-y-1/2 rounded border border-line px-1 font-mono text-[10px] text-ink-3">/</kbd>
        </div>

        <button
          @click="showApiGuide = true"
          title="Agent 派工與 API 接入指南"
          class="h-8 px-2.5 flex items-center gap-1.5 rounded-lg border border-line text-ink-2 hover:bg-raised hover:text-ink text-xs font-medium transition-colors"
        >
          <span>📖</span>
          <span>接入指南</span>
        </button>
        <button
          @click="fetchTasks"
          title="重新整理"
          class="h-8 w-8 flex items-center justify-center rounded-lg border border-line text-ink-2 hover:bg-raised hover:text-ink"
        >
          <Icon name="refresh" :size="14" :class="loading && 'animate-spin'" />
        </button>
        <button
          @click="showCreateModal = true"
          title="新增任務 (N)"
          class="h-8 px-3 flex items-center gap-1.5 rounded-lg bg-accent text-accent-ink text-xs font-semibold hover:opacity-90 transition-opacity"
        >
          <Icon name="plus" :size="14" />
          新增任務
        </button>
      </header>

      <!-- Error Banner -->
      <div v-if="error" class="shrink-0 flex items-center gap-2 border-b border-rose-500/25 bg-rose-500/10 px-5 py-2 text-xs text-rose-700 dark:text-rose-300">
        <span class="h-1.5 w-1.5 rounded-full bg-rose-500"></span>
        <span class="flex-1">無法連線：{{ error }}（請確認後端是否正在監聽 http://127.0.0.1:3000）</span>
        <button @click="fetchTasks" class="font-medium underline-offset-2 hover:underline">重試</button>
        <button @click="error = null" class="ml-1 opacity-70 hover:opacity-100"><Icon name="x" :size="14" /></button>
      </div>

      <div class="flex-1 flex min-h-0">
        <!-- Kanban Columns -->
        <main class="flex-1 min-w-0 overflow-x-auto p-5 flex gap-4">
          <section
            v-for="col in columns"
            :key="col.status"
            class="w-[300px] shrink-0 flex flex-col min-h-0"
          >
            <div class="flex items-center gap-2 px-1 pb-3">
              <span :class="['h-2 w-2 rounded-full', col.meta.dot, col.status === 'running' && 'animate-pulse']"></span>
              <span class="text-xs font-semibold text-ink">{{ col.meta.label }}</span>
              <span class="text-[11px] text-ink-3">{{ col.meta.en }}</span>
              <span class="ml-auto rounded-md bg-raised px-1.5 font-mono text-[11px] text-ink-2">{{ col.tasks.length }}</span>
            </div>

            <div class="flex-1 min-h-0 overflow-y-auto flex flex-col gap-2 pb-2 -mx-1 px-1">
              <TaskCard
                v-for="task in col.tasks"
                :key="task.id"
                :task="task"
                :isSelected="task.id === selectedTaskId"
                @select="handleSelectTask"
                @play="(agent) => handlePlay(task, agent)"
              />
              <div
                v-if="col.tasks.length === 0"
                class="rounded-lg border border-dashed border-line py-8 text-center text-xs text-ink-3"
              >
                {{ search ? "沒有符合的任務" : col.empty }}
              </div>
            </div>
          </section>
        </main>

        <!-- Right Drawer for Live Logs & Details -->
        <TaskDetailDrawer
          :task="selectedTask"
          :logs="activeTaskLogs"
          @close="selectedTaskId = null"
        />
      </div>
    </div>

    <!-- Create Task Modal -->
    <CreateTaskModal
      v-if="showCreateModal"
      @create="handleCreateTask"
      @close="showCreateModal = false"
    />

    <!-- API & Agent Integration Guide Modal -->
    <ApiGuideModal
      v-if="showApiGuide"
      @close="showApiGuide = false"
    />
  </div>
</template>
