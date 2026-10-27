<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import type { CliSession, CreateTaskPayload, Task } from "../types";
import { relativeTime } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  fetchSessions: () => Promise<CliSession[]>;
  createTask: (payload: CreateTaskPayload) => Promise<Task>;
}>();
const emit = defineEmits<{ (e: "close"): void }>();

const sessions = ref<CliSession[]>([]);
const loading = ref(true);
const error = ref<string | null>(null);
const query = ref("");
const openId = ref<string | null>(null);
const prompt = ref("");
const busy = ref(false);
// 最近仍有寫入，代表原 session 可能還開著；同時接續會兩邊寫進同一份對話
const LIVE_MS = 5 * 60 * 1000;

const shown = computed(() => {
  const q = query.value.trim().toLowerCase();
  return q ? sessions.value.filter((s) => `${s.title} ${s.cwd}`.toLowerCase().includes(q)) : sessions.value;
});
const isLive = (s: CliSession) => Date.now() - new Date(s.updated_at).getTime() < LIVE_MS;

onMounted(async () => {
  try {
    sessions.value = await props.fetchSessions();
  } catch (e: any) {
    error.value = e.message || "讀取失敗";
  } finally {
    loading.value = false;
  }
});

const toggle = (s: CliSession) => {
  openId.value = openId.value === s.session_id ? null : s.session_id;
  prompt.value = "";
};

const resume = async (s: CliSession) => {
  if (!prompt.value.trim() || busy.value) return;
  busy.value = true;
  try {
    await props.createTask({
      title: `接續：${s.title}`,
      prompt: prompt.value.trim(),
      creator: "human",
      assigned_agent: s.agent,
      workdir: s.cwd,
      session_id: s.session_id,
    });
    emit("close");
  } catch (e: any) {
    error.value = e.message || "建立任務失敗";
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
          <h3 class="text-sm font-semibold text-ink">CLI Sessions</h3>
          <p class="text-[11px] text-ink-3">本機 claude 既有的 session（含非看板建立的），可接續成看板任務</p>
        </div>
        <button @click="emit('close')" class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink">
          <Icon name="x" />
        </button>
      </div>

      <div class="border-b border-line px-5 py-2.5">
        <input
          v-model="query"
          placeholder="搜尋標題或目錄"
          class="w-full rounded-lg border border-line bg-canvas px-3 py-1.5 text-xs text-ink outline-none focus:border-accent"
        />
      </div>

      <div class="flex-1 overflow-y-auto p-3 text-xs text-ink">
        <p v-if="loading" class="p-4 text-center text-ink-3">讀取中…</p>
        <p v-else-if="error" class="p-4 text-center text-rose-500">{{ error }}</p>
        <p v-else-if="!shown.length" class="p-4 text-center text-ink-3">沒有 session</p>
        <div v-for="s in shown" :key="s.session_id" class="mb-1.5 rounded-lg border border-line">
          <button @click="toggle(s)" class="flex w-full items-start justify-between gap-3 px-3 py-2 text-left hover:bg-raised">
            <div class="min-w-0">
              <div class="truncate font-medium">{{ s.title }}</div>
              <div class="truncate font-mono text-[11px] text-ink-3">{{ s.cwd }}</div>
            </div>
            <span class="shrink-0 text-[11px] text-ink-3">{{ relativeTime(s.updated_at) }}</span>
          </button>
          <div v-if="openId === s.session_id" class="flex flex-col gap-2 border-t border-line px-3 py-2.5">
            <p v-if="isLive(s)" class="text-[11px] text-amber-500">
              這個 session 5 分鐘內仍有活動，可能還開在其他終端機；同時接續會寫進同一份對話。
            </p>
            <textarea
              v-model="prompt"
              rows="3"
              placeholder="接續的指示"
              class="w-full resize-none rounded-lg border border-line bg-canvas px-3 py-2 text-xs outline-none focus:border-accent"
            />
            <div class="flex items-center justify-between">
              <span class="font-mono text-[10px] text-ink-3">{{ s.session_id }}</span>
              <button
                @click="resume(s)"
                :disabled="!prompt.trim() || busy"
                class="rounded-lg bg-accent px-3 py-1 text-xs font-semibold text-accent-ink disabled:opacity-40"
              >
                建立接續任務
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
