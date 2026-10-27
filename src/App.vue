<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useTasks } from "./composables/useTasks";
import TaskCard from "./components/TaskCard.vue";
import CreateTaskModal from "./components/CreateTaskModal.vue";
import TaskDetailDrawer from "./components/TaskDetailDrawer.vue";
import ApiGuideModal from "./components/ApiGuideModal.vue";
import RunTaskModal, { type RunMode } from "./components/RunTaskModal.vue";
import Icon from "./components/Icon.vue";
import DependencyGraph from "./components/DependencyGraph.vue";
import { notifyEnabled } from "./composables/useNotifications";
import { STATUS_META, agentMeta, relativeTime } from "./ui";
import type { CreateTaskPayload, RunOptions, Task, TaskStatus } from "./types";

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
  agents,
  runners,
  spawnTask,
  dispatchTask,
  cancelTask,
  requeueTask,
  followupTask,
  activeTaskArtifacts,
  fetchWorktree,
  mergeWorktree,
  discardWorktree,
} = useTasks();

const showCreateModal = ref(false);
const showApiGuide = ref(false);
const runTarget = ref<Task | null>(null);
const search = ref("");
// 看板 / 依賴關係圖
const view = ref<"board" | "graph">("board");
const searchInput = ref<HTMLInputElement | null>(null);

const agentFilters = computed(() => [
  { value: "all", label: "全部 Agent", count: tasks.value.length },
  { value: "agy", label: "@agy", count: tasks.value.filter((t) => t.assigned_agent === "agy").length },
  { value: "claude", label: "@claude", count: tasks.value.filter((t) => t.assigned_agent === "claude").length },
  { value: "unassigned", label: "未指定", count: tasks.value.filter((t) => !t.assigned_agent).length },
]);

// 每個 Agent 一列；沒出現過的 Runner 也列出來，方便看出「沒有人在領這個 Agent 的任務」
const runnerRows = computed(() =>
  (agents.value.length ? agents.value.map((a) => a.id) : ["agy", "claude"]).map((id) => ({
    id,
    status: runners.value.find((r) => r.agent_id === id),
  }))
);

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

const handleRun = async (mode: RunMode, opts: RunOptions) => {
  const task = runTarget.value;
  if (!task) return;
  try {
    await (mode === "spawn" ? spawnTask : dispatchTask)(task.id, opts);
    runTarget.value = null;
    // 立即執行時打開詳情抽屜，直接看即時日誌
    if (mode === "spawn") selectTask(task.id);
  } catch (err: any) {
    alert((mode === "spawn" ? "執行失敗: " : "派發失敗: ") + err.message);
  }
};

const handleFollowup = async (prompt: string, mode: RunMode) => {
  const task = selectedTask.value;
  if (!task) return;
  try {
    await followupTask(task.id, prompt, mode);
  } catch (err: any) {
    alert("追加指示失敗: " + err.message);
  }
};

const handleRequeue = async (fresh: boolean) => {
  const task = selectedTask.value;
  if (!task) return;
  try {
    await requeueTask(task.id, fresh);
  } catch (err: any) {
    alert("重新排隊失敗: " + err.message);
  }
};

const handleMergeWorktree = async () => {
  const task = selectedTask.value;
  if (!task || !confirm(`把「${task.title}」的分支 ${task.branch} 合併回 repo 目前的分支？\n未 commit 的變更會先自動 commit；合併成功後移除 worktree 與分支。`)) return;
  try {
    alert(await mergeWorktree(task.id));
  } catch (err: any) {
    alert("合併失敗（repo 未被修改）：\n" + err.message);
  }
};

const handleDiscardWorktree = async () => {
  const task = selectedTask.value;
  if (!task || !confirm(`捨棄分支 ${task.branch} 與其 worktree？其中的變更會被刪除，無法復原。`)) return;
  try {
    await discardWorktree(task.id);
  } catch (err: any) {
    alert("捨棄失敗: " + err.message);
  }
};

const handleCancel = async () => {
  const task = selectedTask.value;
  if (!task || !confirm(`確定取消「${task.title}」？執行中的 Agent 會被中止，依賴它的任務也會一併失敗。`)) return;
  try {
    await cancelTask(task.id);
  } catch (err: any) {
    alert("取消失敗: " + err.message);
  }
};

// 鍵盤快捷鍵：N 新增、/ 搜尋、Esc 關閉
const onKeydown = (e: KeyboardEvent) => {
  const typing = e.target instanceof HTMLElement && ["INPUT", "TEXTAREA", "SELECT"].includes(e.target.tagName);
  if (e.key === "Escape") {
    if (runTarget.value) runTarget.value = null;
    else if (showCreateModal.value) showCreateModal.value = false;
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
          <div class="px-2 mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-3">Runners</div>
          <div
            v-for="r in runnerRows"
            :key="r.id"
            :title="r.status ? `最後出現：${relativeTime(r.status.last_seen)}` : '這次啟動後還沒有 Runner 連線'"
            class="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs"
          >
            <span :class="['h-1.5 w-1.5 rounded-full', r.status?.online ? 'bg-emerald-500' : 'bg-ink-3/40']"></span>
            <span class="text-ink-2">@{{ r.id }}</span>
            <button
              v-if="r.status?.online && r.status.current_task"
              @click="selectTask(r.status.current_task)"
              class="ml-auto font-mono text-[11px] text-accent hover:underline"
              title="正在執行的任務"
            >
              #{{ r.status.current_task.slice(0, 6) }}
            </button>
            <span v-else class="ml-auto text-[11px] text-ink-3">
              {{ !r.status ? "未連線" : r.status.online ? "閒置" : "離線" }}
            </span>
          </div>
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

      <button
        @click="notifyEnabled = !notifyEnabled"
        :title="notifyEnabled ? '任務完成或失敗時發桌面通知（視窗在背景時）' : '桌面通知已關閉'"
        class="border-t border-line px-4 py-2.5 flex items-center gap-2 text-[11px] text-ink-2 hover:bg-raised/60"
      >
        <Icon name="bell" :size="13" :class="!notifyEnabled && 'opacity-40'" />
        <span>桌面通知</span>
        <span
          :class="['ml-auto relative h-3.5 w-6 rounded-full transition-colors', notifyEnabled ? 'bg-accent' : 'bg-raised ring-1 ring-line']"
        >
          <span
            :class="['absolute top-0.5 h-2.5 w-2.5 rounded-full bg-white shadow transition-[left]', notifyEnabled ? 'left-3' : 'left-0.5']"
          ></span>
        </span>
      </button>

      <div class="border-t border-line px-4 py-3 flex items-center gap-2 text-[11px]">
        <span class="relative flex h-2 w-2">
          <span v-if="!error" class="absolute inline-flex h-full w-full rounded-full bg-emerald-500 opacity-60 animate-ping"></span>
          <span :class="['relative inline-flex h-2 w-2 rounded-full', error ? 'bg-rose-500' : 'bg-emerald-500']"></span>
        </span>
        <span class="text-ink-2">{{ error ? "API 離線" : "API 連線中" }}</span>
        <span class="ml-auto font-mono text-ink-3">:18833</span>
      </div>
    </aside>

    <!-- Main -->
    <div class="flex-1 min-w-0 flex flex-col">
      <header class="h-14 shrink-0 border-b border-line bg-surface px-5 flex items-center gap-3">
        <h1 class="shrink-0 whitespace-nowrap text-sm font-semibold">任務看板</h1>
        <span class="shrink-0 whitespace-nowrap text-xs text-ink-3">{{ tasks.length }} 個任務</span>
        <div class="ml-2 flex shrink-0 whitespace-nowrap rounded-lg border border-line p-0.5 text-xs">
          <button
            v-for="v in [{ id: 'board', label: '看板' }, { id: 'graph', label: '關係圖' }] as const"
            :key="v.id"
            @click="view = v.id"
            :class="['rounded-md px-2.5 py-1 font-medium transition-colors', view === v.id ? 'bg-raised text-ink' : 'text-ink-3 hover:text-ink']"
          >
            {{ v.label }}
          </button>
        </div>

        <div class="ml-auto relative w-64 min-w-28 shrink">
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
          class="h-8 px-2.5 shrink-0 whitespace-nowrap flex items-center gap-1.5 rounded-lg border border-line text-ink-2 hover:bg-raised hover:text-ink text-xs font-medium transition-colors"
        >
          <span>📖</span>
          <span>接入指南</span>
        </button>
        <button
          @click="fetchTasks"
          title="重新整理"
          class="h-8 w-8 shrink-0 flex items-center justify-center rounded-lg border border-line text-ink-2 hover:bg-raised hover:text-ink"
        >
          <Icon name="refresh" :size="14" :class="loading && 'animate-spin'" />
        </button>
        <button
          @click="showCreateModal = true"
          title="新增任務 (N)"
          class="h-8 px-3 shrink-0 whitespace-nowrap flex items-center gap-1.5 rounded-lg bg-accent text-accent-ink text-xs font-semibold hover:opacity-90 transition-opacity"
        >
          <Icon name="plus" :size="14" />
          新增任務
        </button>
      </header>

      <!-- Error Banner -->
      <div v-if="error" class="shrink-0 flex items-center gap-2 border-b border-rose-500/25 bg-rose-500/10 px-5 py-2 text-xs text-rose-700 dark:text-rose-300">
        <span class="h-1.5 w-1.5 rounded-full bg-rose-500"></span>
        <span class="flex-1">無法連線：{{ error }}（請確認後端是否正在監聽 http://127.0.0.1:18833）</span>
        <button @click="fetchTasks" class="font-medium underline-offset-2 hover:underline">重試</button>
        <button @click="error = null" class="ml-1 opacity-70 hover:opacity-100"><Icon name="x" :size="14" /></button>
      </div>

      <div class="flex-1 flex min-h-0">
        <DependencyGraph
          v-if="view === 'graph'"
          :tasks="columns.flatMap((c) => c.tasks)"
          :selected-id="selectedTaskId"
          @select="handleSelectTask"
        />

        <!-- Kanban Columns -->
        <main v-else class="flex-1 min-w-0 overflow-x-auto p-5 flex gap-4">
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
                @run="runTarget = task"
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
          :artifacts="activeTaskArtifacts"
          :load-worktree="fetchWorktree"
          @merge-worktree="handleMergeWorktree"
          @discard-worktree="handleDiscardWorktree"
          @cancel="handleCancel"
          @requeue="handleRequeue"
          @followup="handleFollowup"
          @select="selectTask"
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
    <RunTaskModal
      v-if="runTarget"
      :task="runTarget"
      :agents="agents"
      @run="handleRun"
      @close="runTarget = null"
    />

    <ApiGuideModal
      v-if="showApiGuide"
      @close="showApiGuide = false"
    />
  </div>
</template>
