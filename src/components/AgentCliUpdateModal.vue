<script setup lang="ts">
import { ref, onMounted } from "vue";
import type { AgentDef, CliUpdateResult } from "../types";
import { agentMeta } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  agents: AgentDef[];
  updateAgentCli: (agentId: string) => Promise<CliUpdateResult>;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

interface AgentStatusItem {
  id: string;
  label: string;
  loading: boolean;
  result: CliUpdateResult | null;
  error: string | null;
}

const items = ref<AgentStatusItem[]>([]);

onMounted(() => {
  items.value = (props.agents.length ? props.agents : [
    { id: "agy", label: "Antigravity CLI", default_model: "gemini-3.8-flash-low", models: [] },
    { id: "claude", label: "Claude Code", default_model: "sonnet", models: [] },
  ]).map((a) => ({
    id: a.id,
    label: a.label,
    loading: false,
    result: null,
    error: null,
  }));
});

const handleCheckOrUpdate = async (item: AgentStatusItem) => {
  item.loading = true;
  item.error = null;
  try {
    item.result = await props.updateAgentCli(item.id);
  } catch (err: any) {
    item.error = err.message || "更新失敗";
  } finally {
    item.loading = false;
  }
};

const handleCheckAll = async () => {
  for (const item of items.value) {
    await handleCheckOrUpdate(item);
  }
};
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-[2px]">
    <div class="flex max-h-[85vh] w-full max-w-xl flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
      <!-- Header -->
      <div class="flex items-center justify-between border-b border-line px-5 py-3.5">
        <div class="flex items-center gap-2">
          <span class="text-base">🚀</span>
          <div>
            <h3 class="text-sm font-semibold text-ink">Agent CLI 版本與更新檢查</h3>
            <p class="text-[11px] text-ink-3">檢查並升級本地端 agy 與 claude 指令列工具</p>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <button
            type="button"
            @click="handleCheckAll"
            class="rounded-lg bg-accent px-3 py-1 text-xs font-semibold text-accent-ink hover:opacity-90 transition-opacity"
          >
            全部檢查更新
          </button>
          <button
            type="button"
            @click="emit('close')"
            class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink"
          >
            <Icon name="x" />
          </button>
        </div>
      </div>

      <!-- Content -->
      <div class="flex-1 overflow-y-auto p-5 text-xs text-ink flex flex-col gap-4">
        <div
          v-for="item in items"
          :key="item.id"
          class="rounded-xl border border-line bg-surface-alt p-4 transition-all"
        >
          <div class="flex items-center justify-between mb-2">
            <div class="flex items-center gap-2">
              <span :class="['h-2 w-2 rounded-full', agentMeta(item.id).dot]"></span>
              <span class="font-bold text-ink text-sm">@{{ item.id }}</span>
              <span class="text-ink-3 text-xs">({{ item.label }})</span>
            </div>
            <button
              @click="handleCheckOrUpdate(item)"
              :disabled="item.loading"
              class="flex items-center gap-1.5 rounded-lg border border-line bg-surface px-2.5 py-1 text-xs font-medium text-ink hover:bg-raised transition-colors disabled:opacity-40"
            >
              <Icon name="refresh" :size="12" :class="item.loading && 'animate-spin'" />
              <span>{{ item.loading ? "檢查升級中…" : "檢查 / 更新" }}</span>
            </button>
          </div>

          <div v-if="item.result" class="mt-3 flex flex-col gap-2 rounded-lg border border-line bg-surface p-3 font-mono text-[11px]">
            <div class="flex items-center justify-between">
              <span class="text-ink-3">目前本機版本：</span>
              <span class="text-ink font-semibold">{{ item.result.current_version ?? '未安裝' }}</span>
            </div>
            <div class="border-t border-line/60 pt-2 text-ink-2 whitespace-pre-wrap leading-relaxed">
              {{ item.result.output }}
            </div>
          </div>

          <div v-else-if="item.error" class="mt-2 text-rose-500 text-xs">
            {{ item.error }}
          </div>

          <div v-else class="text-[11px] text-ink-3 mt-1">
            點擊「檢查 / 更新」以執行本地 <code class="font-mono text-ink-2">{{ item.id }} update</code>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div class="flex items-center justify-end border-t border-line bg-surface-alt px-5 py-3">
        <button
          @click="emit('close')"
          class="rounded-lg bg-raised px-4 py-1.5 text-xs font-semibold text-ink hover:bg-raised/80 transition-colors"
        >
          關閉
        </button>
      </div>
    </div>
  </div>
</template>
