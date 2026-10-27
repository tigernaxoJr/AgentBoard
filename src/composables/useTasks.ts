import { ref, computed, onMounted, onUnmounted } from "vue";
import type { Task, TaskLog, CreateTaskPayload } from "../types";

const API_BASE = "http://127.0.0.1:3000";

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
      const res = await fetch(`${API_BASE}/api/tasks`);
      if (!res.ok) throw new Error("Failed to fetch tasks");
      tasks.value = (await res.json()) as Task[];
    } catch (e: any) {
      error.value = e.message;
    } finally {
      loading.value = false;
    }
  };

  const createTask = async (payload: CreateTaskPayload) => {
    const res = await fetch(`${API_BASE}/api/tasks`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error("Failed to create task");
    const newTask = (await res.json()) as Task;
    tasks.value.unshift(newTask);
    return newTask;
  };

  const fetchLogs = async (taskId: string) => {
    try {
      const res = await fetch(`${API_BASE}/api/tasks/${taskId}/logs`);
      if (res.ok) {
        activeTaskLogs.value = (await res.json()) as TaskLog[];
      }
    } catch (e) {
      console.error("Failed to fetch logs", e);
    }
  };

  const selectTask = (taskId: string) => {
    selectedTaskId.value = taskId;
    fetchLogs(taskId);
  };

  const claimTaskManually = async (_taskId: string, agentId: string) => {
    const res = await fetch(`${API_BASE}/api/tasks/claim`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ agent_id: agentId }),
    });
    if (res.ok) {
      fetchTasks();
    }
  };

  // 即時監聽後端 SSE 事件
  const initSSE = () => {
    try {
      eventSource = new EventSource(`${API_BASE}/api/events`);

      const handleUpdate = () => {
        fetchTasks();
        if (selectedTaskId.value) {
          fetchLogs(selectedTaskId.value);
        }
      };

      eventSource.addEventListener("task_created", handleUpdate);
      eventSource.addEventListener("task_claimed", handleUpdate);
      eventSource.addEventListener("task_completed", handleUpdate);
      eventSource.addEventListener("task_failed", handleUpdate);
      eventSource.addEventListener("task_recovered", handleUpdate);
      eventSource.addEventListener("log_added", () => {
        if (selectedTaskId.value) {
          fetchLogs(selectedTaskId.value);
        }
      });
    } catch (e) {
      console.warn("SSE could not connect, will rely on polling fallback", e);
    }
  };

  onMounted(() => {
    fetchTasks();
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
    claimTaskManually,
  };
}
