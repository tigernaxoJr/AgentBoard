export type TaskStatus = "queued" | "running" | "completed" | "failed";

export type SupportedAgent = "claude" | "agy" | "all";

export interface Task {
  id: string;
  title: string;
  prompt: string;
  status: TaskStatus;
  assigned_agent: string | null;
  creator: string;
  dependency_ids: string[];
  parent_task_id: string | null;
  last_heartbeat_at: string | null;
  attempts: number;
  priority: number;
  model: string | null;
  workdir: string | null;
  auto_approve: boolean;
  isolate: boolean;
  worktree_path: string | null;
  branch: string | null;
  base_commit: string | null;
  session_id: string | null;
  result: string | null;
  cost_usd: number | null;
  input_tokens: number | null;
  output_tokens: number | null;
  duration_ms: number | null;
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
  dependency_ids?: string[];
  workdir?: string | null;
  model?: string | null;
  auto_approve?: boolean;
  isolate?: boolean;
}

export interface AgentDef {
  id: string;
  label: string;
  default_model: string;
  models: string[];
}

/** 執行設定；欄位省略 = 保留原值，model / workdir 傳空字串 = 清除 */
export interface RunOptions {
  agent_id: string;
  model: string;
  workdir: string;
  auto_approve: boolean;
  isolate: boolean;
}

export interface WorktreeStatus {
  branch: string;
  path: string;
  base_commit: string;
  exists: boolean;
  clean: boolean;
  ahead: number;
  behind: number;
  files_changed: number;
  insertions: number;
  deletions: number;
  commits: string[];
  changes: string[];
  diff_stat: string;
}

export interface TaskTemplate {
  id: string;
  name: string;
  description: string | null;
  title: string;
  prompt: string;
  assigned_agent: string | null;
  model: string | null;
  workdir: string | null;
  auto_approve: boolean;
  isolate: boolean;
  priority: number;
  created_at: string;
  updated_at: string;
}

export interface StatsSummary {
  total_tasks: number;
  completed_tasks: number;
  failed_tasks: number;
  running_tasks: number;
  queued_tasks: number;
  total_cost_usd: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_duration_ms: number;
}

export interface AgentStat {
  agent: string;
  task_count: number;
  completed_count: number;
  failed_count: number;
  total_cost_usd: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_duration_ms: number;
}

export interface WorkdirStat {
  workdir: string;
  task_count: number;
  total_cost_usd: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_duration_ms: number;
}

export interface DailyStat {
  date: string;
  task_count: number;
  total_cost_usd: number;
  total_tokens: number;
}

export interface FullStatsResponse {
  summary: StatsSummary;
  by_agent: AgentStat[];
  by_workdir: WorkdirStat[];
  by_date: DailyStat[];
}

export interface RunnerStatus {
  agent_id: string;
  last_seen: string;
  current_task: string | null;
  online: boolean;
}
