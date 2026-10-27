<script setup lang="ts">
import { ref, onMounted } from "vue";
import type { FullStatsResponse } from "../types";
import Icon from "./Icon.vue";

const props = defineEmits<{
  (e: "close"): void;
}>();

const propsData = defineProps<{
  fetchStats: (from?: string, to?: string) => Promise<FullStatsResponse | null>;
}>();

const loading = ref(true);
const stats = ref<FullStatsResponse | null>(null);
const timeRange = ref<"7d" | "30d" | "all">("30d");

const loadStats = async () => {
  loading.value = true;
  let from: string | undefined;
  const now = new Date();
  if (timeRange.value === "7d") {
    const d = new Date(now.getTime() - 7 * 86400000);
    from = d.toISOString().slice(0, 10);
  } else if (timeRange.value === "30d") {
    const d = new Date(now.getTime() - 30 * 86400000);
    from = d.toISOString().slice(0, 10);
  }
  stats.value = await propsData.fetchStats(from);
  loading.value = false;
};

onMounted(loadStats);

const formatNumber = (num: number) => num.toLocaleString();
const formatDuration = (ms: number) => {
  if (ms < 1000) return `${ms}ms`;
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}秒`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  return `${mins}分${remSecs}秒`;
};
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-[2px]">
    <div class="flex max-h-[88vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
      <!-- Header -->
      <div class="flex items-center justify-between border-b border-line px-6 py-4">
        <div class="flex items-center gap-3">
          <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-accent-soft text-accent">
            📊
          </div>
          <div>
            <h3 class="text-base font-semibold text-ink">統計與花費看板</h3>
            <p class="text-xs text-ink-3">Token 用量、成本與執行效能指標</p>
          </div>
        </div>
        <div class="flex items-center gap-3">
          <!-- Time range tabs -->
          <div class="flex rounded-lg bg-raised p-0.5 text-xs">
            <button
              v-for="r in (['7d', '30d', 'all'] as const)"
              :key="r"
              @click="timeRange = r; loadStats()"
              :class="[
                'rounded-md px-3 py-1 font-medium transition-colors',
                timeRange === r ? 'bg-surface text-ink shadow-sm' : 'text-ink-2 hover:text-ink'
              ]"
            >
              {{ r === '7d' ? '近 7 天' : r === '30d' ? '近 30 天' : '全部' }}
            </button>
          </div>
          <button
            @click="$emit('close')"
            class="flex h-8 w-8 items-center justify-center rounded-lg text-ink-3 hover:bg-raised hover:text-ink"
          >
            <Icon name="x" />
          </button>
        </div>
      </div>

      <!-- Body -->
      <div class="flex-1 overflow-y-auto p-6 flex flex-col gap-6 text-sm text-ink">
        <div v-if="loading" class="flex h-64 items-center justify-center text-ink-3 text-xs">
          讀取統計數據中…
        </div>

        <div v-else-if="!stats" class="flex h-64 items-center justify-center text-rose-500 text-xs">
          統計讀取失敗，請稍後再試
        </div>

        <template v-else-if="stats">
          <!-- Top KPI Cards -->
          <div class="grid grid-cols-2 gap-4 sm:grid-cols-4">
            <div class="rounded-xl border border-line bg-surface-alt p-4">
              <span class="text-[11px] font-semibold text-ink-3 uppercase tracking-wider">總任務數</span>
              <div class="mt-2 text-2xl font-bold text-ink">
                {{ formatNumber(stats.summary.total_tasks) }}
              </div>
              <div class="mt-1 flex items-center gap-2 text-[11px] text-ink-2">
                <span class="text-emerald-500 font-medium">✓ {{ stats.summary.completed_tasks }}</span>
                <span class="text-rose-500 font-medium">✗ {{ stats.summary.failed_tasks }}</span>
                <span class="text-amber-500 font-medium">⧗ {{ stats.summary.running_tasks }}</span>
              </div>
            </div>

            <div class="rounded-xl border border-line bg-surface-alt p-4">
              <span class="text-[11px] font-semibold text-ink-3 uppercase tracking-wider">估算花費 (USD)</span>
              <div class="mt-2 text-2xl font-bold text-ink">
                ${{ stats.summary.total_cost_usd.toFixed(4) }}
              </div>
              <div class="mt-1 text-[11px] text-ink-3">
                主要來自 Claude API
              </div>
            </div>

            <div class="rounded-xl border border-line bg-surface-alt p-4">
              <span class="text-[11px] font-semibold text-ink-3 uppercase tracking-wider">總 Token 消耗</span>
              <div class="mt-2 text-2xl font-bold text-ink">
                {{ formatNumber(stats.summary.total_input_tokens + stats.summary.total_output_tokens) }}
              </div>
              <div class="mt-1 flex gap-2 text-[11px] text-ink-3">
                <span>入: {{ formatNumber(stats.summary.total_input_tokens) }}</span>
                <span>出: {{ formatNumber(stats.summary.total_output_tokens) }}</span>
              </div>
            </div>

            <div class="rounded-xl border border-line bg-surface-alt p-4">
              <span class="text-[11px] font-semibold text-ink-3 uppercase tracking-wider">總執行耗時</span>
              <div class="mt-2 text-2xl font-bold text-ink">
                {{ formatDuration(stats.summary.total_duration_ms) }}
              </div>
              <div class="mt-1 text-[11px] text-ink-3">
                平均: {{ stats.summary.completed_tasks > 0 ? formatDuration(Math.round(stats.summary.total_duration_ms / stats.summary.completed_tasks)) : '0ms' }} / 單
              </div>
            </div>
          </div>

          <!-- Section: By Agent -->
          <div>
            <h4 class="mb-3 text-xs font-semibold uppercase tracking-wider text-ink-3">依 Agent 統計</h4>
            <div class="overflow-hidden rounded-xl border border-line">
              <table class="w-full text-left text-xs">
                <thead class="border-b border-line bg-surface-alt text-ink-3 font-medium">
                  <tr>
                    <th class="px-4 py-2.5">Agent</th>
                    <th class="px-4 py-2.5">總任務</th>
                    <th class="px-4 py-2.5">成功 / 失敗</th>
                    <th class="px-4 py-2.5">Input Tokens</th>
                    <th class="px-4 py-2.5">Output Tokens</th>
                    <th class="px-4 py-2.5">花費 (USD)</th>
                    <th class="px-4 py-2.5">總耗時</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-line">
                  <tr v-for="a in stats.by_agent" :key="a.agent" class="hover:bg-raised/40">
                    <td class="px-4 py-3 font-semibold text-ink">
                      @{{ a.agent }}
                    </td>
                    <td class="px-4 py-3 text-ink-2">{{ a.task_count }}</td>
                    <td class="px-4 py-3">
                      <span class="text-emerald-500 font-medium">{{ a.completed_count }}</span>
                      /
                      <span class="text-rose-500 font-medium">{{ a.failed_count }}</span>
                    </td>
                    <td class="px-4 py-3 text-ink-2">{{ formatNumber(a.total_input_tokens) }}</td>
                    <td class="px-4 py-3 text-ink-2">{{ formatNumber(a.total_output_tokens) }}</td>
                    <td class="px-4 py-3 font-mono text-ink">
                      ${{ a.total_cost_usd.toFixed(4) }}
                    </td>
                    <td class="px-4 py-3 text-ink-2">{{ formatDuration(a.total_duration_ms) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>

          <!-- Section: By Workdir -->
          <div v-if="stats.by_workdir.length > 0">
            <h4 class="mb-3 text-xs font-semibold uppercase tracking-wider text-ink-3">依工作目錄統計 (Top 15)</h4>
            <div class="overflow-hidden rounded-xl border border-line">
              <table class="w-full text-left text-xs">
                <thead class="border-b border-line bg-surface-alt text-ink-3 font-medium">
                  <tr>
                    <th class="px-4 py-2.5">工作目錄 (workdir)</th>
                    <th class="px-4 py-2.5">任務數</th>
                    <th class="px-4 py-2.5">總 Tokens</th>
                    <th class="px-4 py-2.5">花費 (USD)</th>
                    <th class="px-4 py-2.5">耗時</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-line font-mono">
                  <tr v-for="w in stats.by_workdir" :key="w.workdir" class="hover:bg-raised/40">
                    <td class="px-4 py-2.5 text-ink truncate max-w-xs" :title="w.workdir">
                      {{ w.workdir }}
                    </td>
                    <td class="px-4 py-2.5 text-ink-2">{{ w.task_count }}</td>
                    <td class="px-4 py-2.5 text-ink-2">{{ formatNumber(w.total_input_tokens + w.total_output_tokens) }}</td>
                    <td class="px-4 py-2.5 text-ink">${{ w.total_cost_usd.toFixed(4) }}</td>
                    <td class="px-4 py-2.5 text-ink-2 font-sans">{{ formatDuration(w.total_duration_ms) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </template>
      </div>

      <!-- Footer -->
      <div class="flex items-center justify-end border-t border-line bg-surface-alt px-6 py-3">
        <button
          @click="$emit('close')"
          class="rounded-lg bg-raised px-4 py-1.5 text-xs font-semibold text-ink hover:bg-raised/80 transition-colors"
        >
          關閉
        </button>
      </div>
    </div>
  </div>
</template>
