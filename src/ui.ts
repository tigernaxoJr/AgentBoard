import type { TaskStatus } from "./types";

export const STATUS_META: Record<TaskStatus, { label: string; en: string; dot: string; text: string; soft: string }> = {
  pending_approval: {
    label: "待核准",
    en: "Pending approval",
    dot: "bg-violet-500",
    text: "text-violet-600 dark:text-violet-400",
    soft: "bg-violet-500/10 text-violet-700 dark:text-violet-300 ring-violet-500/25",
  },
  queued: {
    label: "待領取",
    en: "Queued",
    dot: "bg-amber-500",
    text: "text-amber-600 dark:text-amber-400",
    soft: "bg-amber-500/10 text-amber-700 dark:text-amber-300 ring-amber-500/25",
  },
  running: {
    label: "執行中",
    en: "Running",
    dot: "bg-sky-500",
    text: "text-sky-600 dark:text-sky-400",
    soft: "bg-sky-500/10 text-sky-700 dark:text-sky-300 ring-sky-500/25",
  },
  completed: {
    label: "已完成",
    en: "Completed",
    dot: "bg-emerald-500",
    text: "text-emerald-600 dark:text-emerald-400",
    soft: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 ring-emerald-500/25",
  },
  failed: {
    label: "失敗 / 逾時",
    en: "Failed",
    dot: "bg-rose-500",
    text: "text-rose-600 dark:text-rose-400",
    soft: "bg-rose-500/10 text-rose-700 dark:text-rose-300 ring-rose-500/25",
  },
};

export const agentMeta = (agent: string | null) => {
  switch (agent?.toLowerCase()) {
    case "claude":
      return { dot: "bg-orange-500", chip: "bg-orange-500/10 text-orange-700 dark:text-orange-300 ring-orange-500/25" };
    case "agy":
      return { dot: "bg-teal-500", chip: "bg-teal-500/10 text-teal-700 dark:text-teal-300 ring-teal-500/25" };
    default:
      return { dot: "bg-ink-3", chip: "bg-raised text-ink-2 ring-line-strong" };
  }
};

export const relativeTime = (iso: string | null) => {
  if (!iso) return "—";
  const sec = Math.round((Date.now() - new Date(iso).getTime()) / 1000);
  if (sec < 45) return "剛剛";
  if (sec < 3600) return `${Math.round(sec / 60)} 分鐘前`;
  if (sec < 86400) return `${Math.round(sec / 3600)} 小時前`;
  return `${Math.round(sec / 86400)} 天前`;
};

/** 花費 / token / 耗時的精簡顯示 */
export const formatCost = (usd: number | null) => (usd == null ? null : usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`);
export const formatTokens = (n: number | null) =>
  n == null ? "—" : n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`;
export const formatDuration = (ms: number | null) => {
  if (ms == null) return "—";
  const sec = Math.round(ms / 1000);
  return sec < 60 ? `${sec} 秒` : `${Math.floor(sec / 60)} 分 ${sec % 60} 秒`;
};

/** 從結果文字取出 Agent 結尾附的 ```ask {question, options} 區塊；回傳問題與去掉區塊後的內文 */
export const parseAsk = (result: string | null) => {
  const m = result?.match(/```ask\s*([\s\S]*?)```/);
  if (!result || !m) return null;
  try {
    const { question, options } = JSON.parse(m[1]);
    if (typeof question !== "string" || !Array.isArray(options)) return null;
    return { question, options: options.map(String), body: result.replace(m[0], "").trim() };
  } catch {
    return null;
  }
};
