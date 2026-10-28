<script setup lang="ts">
import { ref, watch, onMounted } from "vue";
import type { AgentMessage } from "../types";
import { relativeTime } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  fetchMessages: () => Promise<AgentMessage[]>;
  sendMessage: (receiver: string, payload: string, taskId?: string) => Promise<void>;
  /** 有新訊息時變動 */
  tick: number;
  agentIds: string[];
}>();
const emit = defineEmits<{ (e: "close"): void }>();

const messages = ref<AgentMessage[]>([]);
const loading = ref(true);
const error = ref<string | null>(null);
const receiver = ref("*");
const text = ref("");
const busy = ref(false);

const load = async () => {
  try {
    messages.value = await props.fetchMessages();
    error.value = null;
  } catch (e: any) {
    error.value = e.message || "讀取失敗";
  } finally {
    loading.value = false;
  }
};
onMounted(load);
watch(() => props.tick, load);

const send = async () => {
  if (!text.value.trim() || !receiver.value.trim() || busy.value) return;
  busy.value = true;
  try {
    await props.sendMessage(receiver.value.trim(), text.value.trim());
    text.value = "";
  } catch (e: any) {
    error.value = e.message || "發送失敗";
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-[2px]">
    <div class="flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
      <div class="flex items-center justify-between border-b border-line px-5 py-3.5">
        <div>
          <h3 class="text-sm font-semibold text-ink">Agent 訊息</h3>
          <p class="text-[11px] text-ink-3">Agent 之間透過 MCP / REST 互傳的訊息；收件者填 * 代表廣播</p>
        </div>
        <button @click="emit('close')" class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink">
          <Icon name="x" />
        </button>
      </div>

      <div class="flex-1 overflow-y-auto p-3 text-xs text-ink">
        <p v-if="loading" class="p-4 text-center text-ink-3">讀取中…</p>
        <p v-else-if="!messages.length" class="p-4 text-center text-ink-3">還沒有訊息</p>
        <div v-for="m in messages" :key="m.id" class="mb-1.5 rounded-lg border border-line px-3 py-2">
          <div class="flex items-center justify-between gap-3 text-[11px] text-ink-3">
            <span>
              <span class="font-medium text-ink">{{ m.sender }}</span> → {{ m.receiver === "*" ? "所有人" : m.receiver }}
              <span v-if="m.task_id" class="ml-1 font-mono">#{{ m.task_id.slice(0, 8) }}</span>
            </span>
            <span class="shrink-0">
              {{ m.receiver !== "*" ? (m.is_read ? "已讀 · " : "未讀 · ") : "" }}{{ relativeTime(m.created_at) }}
            </span>
          </div>
          <div class="mt-1 whitespace-pre-wrap break-words">{{ m.payload }}</div>
        </div>
      </div>

      <p v-if="error" class="px-5 pb-1 text-[11px] text-rose-500">{{ error }}</p>
      <div class="flex gap-2 border-t border-line px-5 py-3">
        <input
          v-model="receiver"
          list="message-receivers"
          placeholder="收件者"
          class="w-28 rounded-lg border border-line bg-canvas px-3 py-1.5 text-xs text-ink outline-none focus:border-accent"
        />
        <datalist id="message-receivers">
          <option value="*" />
          <option v-for="id in agentIds" :key="id" :value="id" />
        </datalist>
        <input
          v-model="text"
          @keydown.enter="send"
          placeholder="以 human 身分發送訊息"
          class="min-w-0 flex-1 rounded-lg border border-line bg-canvas px-3 py-1.5 text-xs text-ink outline-none focus:border-accent"
        />
        <button
          @click="send"
          :disabled="!text.trim() || !receiver.trim() || busy"
          class="rounded-lg bg-accent px-3 py-1 text-xs font-semibold text-accent-ink disabled:opacity-40"
        >
          發送
        </button>
      </div>
    </div>
  </div>
</template>
