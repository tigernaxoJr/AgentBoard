import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { notifyTask } from "./useNotifications";
import type { AgentDef, Artifact, CreateTaskPayload, RunnerStatus, RunOptions, Task, TaskLog, WorktreeStatus } from "../types";

interface ApiConfig {
  base_url: string;
  token: string;
}

// API 需要 token：在 Tauri 視窗內向 Rust 端取得；純瀏覽器開發時改用 VITE_API_TOKEN
const apiConfig: Promise<ApiConfig> = invoke<ApiConfig>("get_api_config").catch(() => ({
  base_url: "http://127.0.0.1:18833",
  token: import.meta.env.VITE_API_TOKEN ?? "",
}));

// 與後端 MAX_LOGS_PER_TASK 一致
export const MAX_LOGS = 2000;

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

  // 方案 B 的 Runner 在線狀態；離線是「太久沒出現」，後端不會主動通知，所以定期重抓
  const runners = ref<RunnerStatus[]>([]);
  const fetchRunners = async () => {
    const res = await api("/api/runners");
    if (res.ok) runners.value = (await res.json()) as RunnerStatus[];
  };
  let runnerTimer: ReturnType<typeof setInterval> | undefined;

  const postTaskAction = async (taskId: string, action: string, body?: object) => {
    const res = await api(`/api/tasks/${taskId}/${action}`, { method: "POST", body: JSON.stringify(body ?? {}) });
    if (!res.ok) throw new Error(await res.text());
    fetchTasks();
  };

  // 方案 A：寫入執行設定，由面板後端立即拉起 Agent 子程序
  const spawnTask = (taskId: string, opts: Partial<RunOptions>) => postTaskAction(taskId, "spawn", opts);
  // 方案 B：寫入執行設定並排到該 Agent 佇列最前面，由 Runner 下次輪詢領取
  const dispatchTask = (taskId: string, opts: Partial<RunOptions>) => postTaskAction(taskId, "dispatch", opts);
  // 排隊中或執行中都能取消；Runner 執行的任務會在下次心跳時中止
  const cancelTask = (taskId: string) => postTaskAction(taskId, "cancel");
  // 失敗的任務重新排隊（因它連鎖失敗的下游任務一併恢復）；fresh 捨棄工作階段，從頭執行
  const requeueTask = (taskId: string, fresh = false) => postTaskAction(taskId, "requeue", { fresh });

  // 追加指示：建立接續原工作階段的新任務，再依 mode 立即執行或派發給 Runner
  const followupTask = async (taskId: string, prompt: string, mode: "spawn" | "dispatch") => {
    const res = await api(`/api/tasks/${taskId}/followup`, { method: "POST", body: JSON.stringify({ prompt }) });
    if (!res.ok) throw new Error(await res.text());
    const child = (await res.json()) as Task;
    tasks.value.unshift(child);
    await (mode === "spawn" ? spawnTask : dispatchTask)(child.id, { agent_id: child.assigned_agent! });
    selectTask(child.id);
  };

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
        fetchRunners();
        if (selectedTaskId.value) {
          fetchLogs(selectedTaskId.value);
          fetchArtifacts(selectedTaskId.value);
        }
      });

      eventSource.addEventListener("task_created", handleUpdate);
      eventSource.addEventListener("task_claimed", handleUpdate);
      eventSource.addEventListener("task_dispatched", handleUpdate);
      // 完成 / 失敗另外發桌面通知（手動取消的不通知）
      const notifyFinished = (ok: boolean) => (e: Event) => {
        const { id, cancelled } = JSON.parse((e as MessageEvent).data).payload as { id: string; cancelled?: boolean };
        const task = tasks.value.find((t) => t.id === id);
        if (!cancelled) notifyTask(ok ? "任務完成" : "任務失敗", `#${id.slice(0, 8)} ${task?.title ?? ""}`.trim());
        fetchTasks();
      };
      eventSource.addEventListener("task_completed", notifyFinished(true));
      eventSource.addEventListener("task_failed", notifyFinished(false));
      eventSource.addEventListener("task_recovered", handleUpdate);
      eventSource.addEventListener("task_requeued", handleUpdate);
      eventSource.addEventListener("worktree_changed", handleUpdate);
      eventSource.addEventListener("runners_changed", fetchRunners);
      eventSource.addEventListener("artifact_created", (e) => {
        const artifact = JSON.parse((e as MessageEvent).data).payload as Artifact;
        if (artifact.task_id === selectedTaskId.value) activeTaskArtifacts.value.unshift(artifact);
      });
      eventSource.addEventListener("log_added", (e) => {
        const log = JSON.parse((e as MessageEvent).data).payload as TaskLog;
        // 只處理目前開著的任務；fetchLogs 與事件可能重疊，以 id 去重
        if (log.task_id !== selectedTaskId.value) return;
        if (activeTaskLogs.value.some((l) => l.id === log.id)) return;
        activeTaskLogs.value.push(log);
        // 與後端相同只保留最近 MAX_LOGS 則
        if (activeTaskLogs.value.length > MAX_LOGS) activeTaskLogs.value.splice(0, activeTaskLogs.value.length - MAX_LOGS);
      });
    } catch (e) {
      console.warn("SSE could not connect, will rely on polling fallback", e);
    }
  };

  onMounted(() => {
    fetchTasks();
    fetchAgents();
    initSSE();
    runnerTimer = setInterval(fetchRunners, 15_000);
  });

  onUnmounted(() => {
    clearInterval(runnerTimer);
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
  };
}
