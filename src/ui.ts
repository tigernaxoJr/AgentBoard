import type { TaskStatus } from "./types";

export const STATUS_META: Record<TaskStatus, { label: string; en: string; dot: string; text: string; soft: string }> = {
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
