<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { AgentDef, RunOptions, Task } from "../types";
import { agentMeta } from "../ui";
import Icon from "./Icon.vue";

export type RunMode = "spawn" | "dispatch";

const props = defineProps<{
  task: Task;
  agents: AgentDef[];
}>();

const emit = defineEmits<{
  (e: "run", mode: RunMode, opts: RunOptions): void;
  (e: "close"): void;
}>();

// 預設帶入任務目前的設定
const agentId = ref(props.task.assigned_agent && props.agents.some((a) => a.id === props.task.assigned_agent) ? props.task.assigned_agent : "agy");
const model = ref(props.task.model ?? "");
const workdir = ref(props.task.workdir ?? "");
const autoApprove = ref(props.task.auto_approve);
const isolate = ref(props.task.isolate);
const mode = ref<RunMode>("spawn");

const agent = computed(() => props.agents.find((a) => a.id === agentId.value));
// 換 Agent 時，原本的模型若不在新 Agent 的清單裡就清掉（回到預設）
watch(agentId, () => {
  if (model.value && !agent.value?.models.includes(model.value)) model.value = "";
});

const modes: { value: RunMode; label: string; hint: string }[] = [
  { value: "spawn", label: "立即在本機執行", hint: "面板直接拉起 Agent，日誌即時顯示" },
  { value: "dispatch", label: "派發給 Runner", hint: "排到佇列最前面，由常駐 Runner 領取" },
];

const submit = () => {
  emit("run", mode.value, {
    agent_id: agentId.value,
    model: model.value.trim(),
    workdir: workdir.value.trim(),
    auto_approve: autoApprove.value,
    isolate: isolate.value && !!workdir.value.trim(),
  });
};

const field = "w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-ink placeholder:text-ink-3 focus:border-accent focus:outline-none focus:ring-3 focus:ring-accent-soft";
const label = "mb-2 block text-[11px] font-semibold uppercase tracking-wider text-ink-3";
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-4 pt-[12vh] backdrop-blur-[2px]" @click.self="emit('close')">
    <form
      @submit.prevent="submit"
      @keydown.ctrl.enter="submit"
      @keydown.meta.enter="submit"
      class="w-full max-w-lg overflow-hidden rounded-xl border border-line bg-surface shadow-2xl"
    >
      <div class="flex items-center justify-between border-b border-line px-5 py-3.5">
        <div class="min-w-0">
          <h3 class="text-sm font-semibold text-ink">執行任務</h3>
          <p class="truncate text-xs text-ink-3">{{ task.title }}</p>
        </div>
        <button type="button" @click="emit('close')" class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink">
          <Icon name="x" />
        </button>
      </div>

      <div class="flex flex-col gap-5 p-5">
        <div>
          <span :class="label">執行方式</span>
          <div class="grid grid-cols-2 gap-2">
            <label
              v-for="m in modes"
              :key="m.value"
              :class="['cursor-pointer rounded-lg border px-3 py-2.5 transition-colors', mode === m.value ? 'border-accent bg-accent-soft' : 'border-line hover:border-line-strong']"
            >
              <input v-model="mode" type="radio" :value="m.value" class="sr-only" />
              <span class="block text-xs font-semibold text-ink">{{ m.label }}</span>
              <span class="mt-0.5 block text-[11px] text-ink-3">{{ m.hint }}</span>
            </label>
          </div>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <div>
            <span :class="label">Agent</span>
            <div class="flex rounded-lg bg-raised p-0.5 text-xs">
              <button
                v-for="a in agents"
                :key="a.id"
                type="button"
                @click="agentId = a.id"
                :title="a.label"
                :class="['flex flex-1 items-center justify-center gap-1.5 rounded-md py-1.5 font-medium transition-colors', agentId === a.id ? 'bg-surface text-ink shadow-sm' : 'text-ink-2 hover:text-ink']"
              >
                <span :class="['h-1.5 w-1.5 rounded-full', agentMeta(a.id).dot]"></span>
                @{{ a.id }}
              </button>
            </div>
          </div>
          <div>
            <span :class="label">模型</span>
            <input v-model="model" list="run-model-presets" :placeholder="`預設（${agent?.default_model ?? '—'}）`" :class="[field, 'font-mono']" />
            <datalist id="run-model-presets">
              <option v-for="m in agent?.models ?? []" :key="m" :value="m" />
            </datalist>
          </div>
        </div>

        <div>
          <span :class="label">工作目錄（選填）</span>
          <input v-model="workdir" placeholder="/home/you/projects/app，留空則使用 Runner 的目錄或家目錄" :class="[field, 'font-mono']" />
        </div>

        <label :class="['flex items-start gap-2.5 rounded-lg border border-line px-3 py-2.5', workdir.trim() ? 'cursor-pointer' : 'opacity-50']">
          <input v-model="isolate" type="checkbox" :disabled="!workdir.trim()" class="mt-0.5 accent-indigo-500" />
          <span>
            <span class="block text-xs font-semibold text-ink">在獨立的 git worktree 執行</span>
            <span class="mt-0.5 block text-[11px] text-ink-3">
              在 <code class="font-mono">agent-board/&lt;任務ID&gt;</code> 分支上工作，不影響你的工作目錄；完成後可在詳情中審閱並合併。需要先填工作目錄（git repo 內）。
            </span>
          </span>
        </label>

        <label class="flex cursor-pointer items-start gap-2.5 rounded-lg border border-line px-3 py-2.5">
          <input v-model="autoApprove" type="checkbox" class="mt-0.5 accent-rose-500" />
          <span>
            <span class="block text-xs font-semibold text-ink">完整權限（自動核准所有工具）</span>
            <span class="mt-0.5 block text-[11px] text-ink-3">
              未勾選時只自動核准檔案編輯。勾選後 Agent 可不經詢問執行任何指令，請只用在信任的任務上。
            </span>
          </span>
        </label>
      </div>

      <div class="flex items-center justify-between border-t border-line bg-raised/50 px-5 py-3">
        <span class="text-[11px] text-ink-3"><kbd class="font-mono">Ctrl</kbd> + <kbd class="font-mono">Enter</kbd> 送出</span>
        <div class="flex gap-2">
          <button type="button" @click="emit('close')" class="rounded-lg px-3.5 py-1.5 text-xs font-medium text-ink-2 hover:bg-raised hover:text-ink">取消</button>
          <button
            type="submit"
            :disabled="!agent"
            class="flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-1.5 text-xs font-semibold text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            <Icon name="play" :size="10" />
            {{ mode === "spawn" ? "立即執行" : "派發" }}
          </button>
        </div>
      </div>
    </form>
  </div>
</template>
