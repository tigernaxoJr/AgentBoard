export type TaskStatus = "queued" | "running" | "completed" | "failed";

export type SupportedAgent = "claude" | "agy" | "all";

export interface Task {
  id: string;
  title: string;
  prompt: string;
  status: TaskStatus;
  assigned_agent: string | null;
  creator: string;
  depends_on_task_id: string | null;
  last_heartbeat_at: string | null;
  attempts: number;
  priority: number;
  created_at: string;
  updated_at: string;
}

export interface TaskLog {
  id: string;
  task_id: string;
  agent_id: string;
  level: string;
  message: string;
  created_at: string;
}

export interface Artifact {
  id: string;
  task_id: string;
  agent_id: string;
  file_path: string;
  summary: string;
  created_at: string;
}

export interface CreateTaskPayload {
  title: string;
  prompt: string;
  creator: string;
  assigned_agent?: string | null;
  depends_on_task_id?: string | null;
}
