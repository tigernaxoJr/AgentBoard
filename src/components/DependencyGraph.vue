<script setup lang="ts">
import { computed } from "vue";
import type { Task } from "../types";
import { STATUS_META, agentMeta } from "../ui";

const props = defineProps<{ tasks: Task[]; selectedId: string | null }>();
const emit = defineEmits<{ (e: "select", task: Task): void }>();

const W = 220;
const H = 66;
const GAP_X = 64;
const GAP_Y = 14;
const GAP_COMPONENT = 36;
const PAD = 24;

interface Edge {
  from: Task;
  to: Task;
  kind: "dep" | "followup";
}

// 只畫有關係的任務：依賴（depends_on）與追加指示（parent）都算一條邊
const graph = computed(() => {
  const byId = new Map(props.tasks.map((t) => [t.id, t]));
  const edges: Edge[] = [];
  for (const t of props.tasks) {
    for (const id of t.dependency_ids) {
      const dep = byId.get(id);
      if (dep) edges.push({ from: dep, to: t, kind: "dep" });
    }
    const parent = t.parent_task_id && byId.get(t.parent_task_id);
    if (parent) edges.push({ from: parent, to: t, kind: "followup" });
  }

  // 依連通元件分組，每組從左到右按深度排
  const root = new Map<string, string>();
  const find = (id: string): string => {
    const r = root.get(id) ?? id;
    if (r === id) return id;
    const top = find(r);
    root.set(id, top);
    return top;
  };
  for (const e of edges) root.set(find(e.to.id), find(e.from.id));

  const incoming = new Map<string, Task[]>();
  for (const e of edges) incoming.set(e.to.id, [...(incoming.get(e.to.id) ?? []), e.from]);
  const depthMemo = new Map<string, number>();
  const depth = (t: Task, seen = new Set<string>()): number => {
    if (depthMemo.has(t.id)) return depthMemo.get(t.id)!;
    if (seen.has(t.id)) return 0; // 資料不一致時避免無窮遞迴
    seen.add(t.id);
    const d = Math.max(-1, ...(incoming.get(t.id) ?? []).map((p) => depth(p, seen))) + 1;
    depthMemo.set(t.id, d);
    return d;
  };

  const related = new Set(edges.flatMap((e) => [e.from.id, e.to.id]));
  const components = new Map<string, Task[]>();
  for (const t of props.tasks.filter((t) => related.has(t.id))) {
    const key = find(t.id);
    components.set(key, [...(components.get(key) ?? []), t]);
  }

  const pos = new Map<string, { x: number; y: number }>();
  let offsetY = PAD;
  let maxDepth = 0;
  const groups = [...components.values()].sort((a, b) => oldest(a).localeCompare(oldest(b)));
  for (const group of groups) {
    const columns = new Map<number, Task[]>();
    for (const t of [...group].sort((a, b) => a.created_at.localeCompare(b.created_at))) {
      const d = depth(t);
      maxDepth = Math.max(maxDepth, d);
      columns.set(d, [...(columns.get(d) ?? []), t]);
    }
    let rows = 0;
    for (const [d, list] of columns) {
      list.forEach((t, i) => pos.set(t.id, { x: PAD + d * (W + GAP_X), y: offsetY + i * (H + GAP_Y) }));
      rows = Math.max(rows, list.length);
    }
    offsetY += rows * (H + GAP_Y) - GAP_Y + GAP_COMPONENT;
  }

  const nodes = props.tasks.filter((t) => pos.has(t.id)).map((t) => ({ task: t, ...pos.get(t.id)! }));
  const paths = edges.map((e) => {
    const a = pos.get(e.from.id)!;
    const b = pos.get(e.to.id)!;
    const [x1, y1, x2, y2] = [a.x + W, a.y + H / 2, b.x, b.y + H / 2];
    const mid = (x1 + x2) / 2;
    return { ...e, d: `M${x1},${y1} C${mid},${y1} ${mid},${y2} ${x2},${y2}`, ...edgeStyle(e) };
  });

  return {
    nodes,
    paths,
    width: PAD * 2 + (maxDepth + 1) * W + maxDepth * GAP_X,
    height: Math.max(offsetY - GAP_COMPONENT + PAD, 0),
  };
});

const oldest = (list: Task[]) => list.reduce((m, t) => (t.created_at < m ? t.created_at : m), list[0].created_at);

const EDGE_COLORS = ["text-violet-400", "text-emerald-500", "text-rose-500", "text-amber-500"];

// 依賴邊的顏色代表下游能不能開始：綠 = 前置已完成、紅 = 前置失敗、琥珀虛線 = 還在等
const edgeStyle = (e: Edge) => {
  if (e.kind === "followup") return { color: "text-violet-400", dash: "2 4" };
  if (e.from.status === "completed") return { color: "text-emerald-500", dash: "" };
  if (e.from.status === "failed") return { color: "text-rose-500", dash: "" };
  return { color: "text-amber-500", dash: "6 4" };
};

const byId = computed(() => new Map(props.tasks.map((t) => [t.id, t])));

// 卡住的原因：等前置任務、或因前置失敗被連帶標記失敗
const blockedLabel = (t: Task) => {
  const deps = t.dependency_ids.map((id) => byId.value.get(id)).filter((d): d is Task => !!d);
  if (!deps.length) return null;
  if (t.status === "queued" && deps.some((d) => d.status !== "completed")) return "等待前置";
  if (t.status === "failed" && t.attempts === 0 && deps.some((d) => d.status === "failed")) return "連鎖失敗";
  return null;
};
</script>

<template>
  <main class="flex-1 min-w-0 overflow-auto">
    <div v-if="!graph.nodes.length" class="flex h-full flex-col items-center justify-center gap-1 text-xs text-ink-3">
      <p>目前沒有互相依賴或追加指示的任務</p>
      <p>建立任務時填入「前置任務 ID」，或在已結束的任務上追加指示，這裡就會畫出關係</p>
    </div>

    <template v-else>
      <div class="sticky left-0 top-0 z-10 flex items-center gap-4 px-6 pt-4 text-[11px] text-ink-3">
        <span class="flex shrink-0 items-center gap-1.5 whitespace-nowrap"><span class="h-0.5 w-5 bg-emerald-500"></span>前置已完成</span>
        <span class="flex shrink-0 items-center gap-1.5 whitespace-nowrap"><span class="h-0.5 w-5 border-t-2 border-dashed border-amber-500"></span>等待前置</span>
        <span class="flex shrink-0 items-center gap-1.5 whitespace-nowrap"><span class="h-0.5 w-5 bg-rose-500"></span>前置失敗</span>
        <span class="flex shrink-0 items-center gap-1.5 whitespace-nowrap"><span class="h-0.5 w-5 border-t-2 border-dotted border-violet-400"></span>追加指示</span>
      </div>

      <div class="relative" :style="{ width: `${graph.width}px`, height: `${graph.height}px` }">
        <svg class="absolute inset-0 overflow-visible" :width="graph.width" :height="graph.height" aria-hidden="true">
          <defs>
            <!-- 箭頭顏色跟著邊；每種顏色一個 marker（context-stroke 在舊版 WebKit 不支援） -->
            <marker
              v-for="c in EDGE_COLORS"
              :key="c"
              :id="`arrow-${c}`"
              :class="c"
              viewBox="0 0 8 8"
              refX="7"
              refY="4"
              markerWidth="7"
              markerHeight="7"
              orient="auto"
            >
              <path d="M0,0 L8,4 L0,8 z" fill="currentColor" />
            </marker>
          </defs>
          <path
            v-for="p in graph.paths"
            :key="`${p.from.id}-${p.to.id}-${p.kind}`"
            :d="p.d"
            :class="p.color"
            stroke="currentColor"
            stroke-width="1.75"
            :stroke-dasharray="p.dash"
            fill="none"
            :marker-end="`url(#arrow-${p.color})`"
          />
        </svg>

        <button
          v-for="n in graph.nodes"
          :key="n.task.id"
          @click="emit('select', n.task)"
          :style="{ left: `${n.x}px`, top: `${n.y}px`, width: `${W}px`, height: `${H}px` }"
          :class="[
            'absolute flex flex-col justify-between rounded-lg border bg-surface px-3 py-2 text-left transition-[border-color,box-shadow]',
            n.task.id === selectedId ? 'border-accent shadow-[0_0_0_3px_var(--accent-soft)]' : 'border-line hover:border-line-strong',
          ]"
        >
          <div class="flex items-center gap-1.5 text-[11px]">
            <span :class="['h-2 w-2 shrink-0 rounded-full', STATUS_META[n.task.status].dot, n.task.status === 'running' && 'animate-pulse']"></span>
            <span class="font-mono text-ink-3">#{{ n.task.id.slice(0, 6) }}</span>
            <span v-if="blockedLabel(n.task)" class="rounded bg-raised px-1 text-ink-2">{{ blockedLabel(n.task) }}</span>
            <span v-if="n.task.assigned_agent" class="ml-auto flex items-center gap-1 text-ink-3">
              <span :class="['h-1.5 w-1.5 rounded-full', agentMeta(n.task.assigned_agent).dot]"></span>
              @{{ n.task.assigned_agent }}
            </span>
          </div>
          <div class="truncate text-[12px] font-medium text-ink">{{ n.task.title }}</div>
        </button>
      </div>
    </template>
  </main>
</template>
