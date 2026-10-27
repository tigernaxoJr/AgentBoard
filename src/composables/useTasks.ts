import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AgentDef, Artifact, CreateTaskPayload, RunOptions, Task, TaskLog, WorktreeStatus } from "../types";

interface ApiConfig {
  base_url: string;
  token: string;
}

// API 需要 token：在 Tauri 視窗內向 Rust 端取得；純瀏覽器開發時改用 VITE_API_TOKEN
const apiConfig: Promise<ApiConfig> = invoke<ApiConfig>("get_api_config").catch(() => ({
  base_url: "http://127.0.0.1:18833",
  token: import.meta.env.VITE_API_TOKEN ?? "",
}));

const api = async (path: string, init: RequestInit = {}) => {
  const { base_url, token } = await apiConfig;
  return fetch(`${base_url}${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}`, ...init.headers },
  });
};

export function useTasks() {
  const tasks = ref<Task[]>([]);
  const activeTaskLogs = ref<TaskLog[]>([]);
  const selectedTaskId = ref<string | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const agentFilter = ref<string>("all");

  let eventSource: EventSource | null = null;

  const fetchTasks = async () => {
    loading.value = true;
    error.value = null;
    try {
      const res = await api("/api/tasks");
      if (!res.ok) throw new Error(res.status === 401 ? "API token 無效" : "Failed to fetch tasks");
      tasks.value = (await res.json()) as Task[];
    } catch (e: any) {
      error.value = e.message;
    } finally {
      loading.value = false;
    }
  };

  const createTask = async (payload: CreateTaskPayload) => {
    const res = await api("/api/tasks", {
      method: "POST",
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error("Failed to create task");
    const newTask = (await res.json()) as Task;
    tasks.value.unshift(newTask);
    return newTask;
  };

  const fetchLogs = async (taskId: string) => {
    try {
      const res = await api(`/api/tasks/${taskId}/logs`);
      const logs = res.ok ? ((await res.json()) as TaskLog[]) : null;
      // 回應較慢時使用者可能已切到別的任務
      if (logs && selectedTaskId.value === taskId) activeTaskLogs.value = logs;
    } catch (e) {
      console.error("Failed to fetch logs", e);
    }
  };

  const activeTaskArtifacts = ref<Artifact[]>([]);
  const fetchArtifacts = async (taskId: string) => {
    const res = await api(`/api/tasks/${taskId}/artifacts`);
    const artifacts = res.ok ? ((await res.json()) as Artifact[]) : null;
    if (artifacts && selectedTaskId.value === taskId) activeTaskArtifacts.value = artifacts;
  };

  const selectTask = (taskId: string) => {
    selectedTaskId.value = taskId;
    activeTaskLogs.value = [];
    activeTaskArtifacts.value = [];
    fetchLogs(taskId);
    fetchArtifacts(taskId);
  };

  const agents = ref<AgentDef[]>([]);
  const fetchAgents = async () => {
    const res = await api("/api/agents");
    if (res.ok) agents.value = (await res.json()) as AgentDef[];
  };

  const postTaskAction = async (taskId: string, action: string, body?: object) => {
    const res = await api(`/api/tasks/${taskId}/${action}`, { method: "POST", body: JSON.stringify(body ?? {}) });
    if (!res.ok) throw new Error(await res.text());
    fetchTasks();
  };

  // 方案 A：寫入執行設定，由面板後端立即拉起 Agent 子程序
  const spawnTask = (taskId: string, opts: RunOptions) => postTaskAction(taskId, "spawn", opts);
  // 方案 B：寫入執行設定並排到該 Agent 佇列最前面，由 Runner 下次輪詢領取
  const dispatchTask = (taskId: string, opts: RunOptions) => postTaskAction(taskId, "dispatch", opts);
  // 排隊中或執行中都能取消；Runner 執行的任務會在下次心跳時中止
  const cancelTask = (taskId: string) => postTaskAction(taskId, "cancel");
  // 失敗的任務重新排隊（因它連鎖失敗的下游任務一併恢復）
  const requeueTask = (taskId: string) => postTaskAction(taskId, "requeue");

  // 隔離執行的 worktree：查看狀態、合併回 repo、捨棄
  const fetchWorktree = async (taskId: string) => {
    const res = await api(`/api/tasks/${taskId}/worktree`);
    if (!res.ok) throw new Error(await res.text());
    return (await res.json()) as WorktreeStatus;
  };
  const mergeWorktree = async (taskId: string) => {
    const res = await api(`/api/tasks/${taskId}/worktree/merge`, { method: "POST" });
    if (!res.ok) throw new Error(await res.text());
    fetchTasks();
    return ((await res.json()) as { message: string }).message;
  };
  const discardWorktree = (taskId: string) => postTaskAction(taskId, "worktree/discard");

  // 即時監聽後端 SSE 事件
  const initSSE = async () => {
    try {
      const { base_url, token } = await apiConfig;
      eventSource = new EventSource(`${base_url}/api/events?token=${token}`);

      // 狀態變更只重抓任務列表；日誌由 log_added 事件逐則附加，不再整份重抓
      const handleUpdate = () => fetchTasks();
      // 斷線重連期間可能漏掉事件，連上時整份同步一次
      eventSource.addEventListener("open", () => {
        fetchTasks();
        if (selectedTaskId.value) {
          fetchLogs(selectedTaskId.value);
          fetchArtifacts(selectedTaskId.value);
        }
      });

      eventSource.addEventListener("task_created", handleUpdate);
      eventSource.addEventListener("task_claimed", handleUpdate);
      eventSource.addEventListener("task_dispatched", handleUpdate);
      eventSource.addEventListener("task_completed", handleUpdate);
      eventSource.addEventListener("task_failed", handleUpdate);
      eventSource.addEventListener("task_recovered", handleUpdate);
      eventSource.addEventListener("task_requeued", handleUpdate);
      eventSource.addEventListener("worktree_changed", handleUpdate);
      eventSource.addEventListener("artifact_created", (e) => {
        const artifact = JSON.parse((e as MessageEvent).data).payload as Artifact;
        if (artifact.task_id === selectedTaskId.value) activeTaskArtifacts.value.unshift(artifact);
      });
      eventSource.addEventListener("log_added", (e) => {
        const log = JSON.parse((e as MessageEvent).data).payload as TaskLog;
        // 只處理目前開著的任務；fetchLogs 與事件可能重疊，以 id 去重
        if (log.task_id !== selectedTaskId.value) return;
        if (!activeTaskLogs.value.some((l) => l.id === log.id)) activeTaskLogs.value.push(log);
      });
    } catch (e) {
      console.warn("SSE could not connect, will rely on polling fallback", e);
    }
  };

  onMounted(() => {
    fetchTasks();
    fetchAgents();
    initSSE();
  });

  onUnmounted(() => {
    if (eventSource) {
      eventSource.close();
    }
  });

  // 按 Agent 篩選
  const filteredTasks = computed<Task[]>(() => {
    if (agentFilter.value === "all") return tasks.value;
    return tasks.value.filter(
      (t: Task) => t.assigned_agent === agentFilter.value || (!t.assigned_agent && agentFilter.value === "unassigned")
    );
  });

  const queuedTasks = computed<Task[]>(() => filteredTasks.value.filter((t: Task) => t.status === "queued"));
  const runningTasks = computed<Task[]>(() => filteredTasks.value.filter((t: Task) => t.status === "running"));
  const completedTasks = computed<Task[]>(() => filteredTasks.value.filter((t: Task) => t.status === "completed"));
  const failedTasks = computed<Task[]>(() => filteredTasks.value.filter((t: Task) => t.status === "failed"));

  const selectedTask = computed<Task | null>(() => tasks.value.find((t: Task) => t.id === selectedTaskId.value) || null);

  return {
    tasks,
    filteredTasks,
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
    spawnTask,
    dispatchTask,
    cancelTask,
    requeueTask,
    activeTaskArtifacts,
    fetchWorktree,
    mergeWorktree,
    discardWorktree,
  };
}
