<script setup lang="ts">
import { ref, watch } from "vue";
import type { CreateTaskPayload, TaskTemplate } from "../types";
import { agentMeta } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  initialTemplate?: TaskTemplate | null;
}>();

const emit = defineEmits<{
  (e: "create", payload: CreateTaskPayload): void;
  (e: "close"): void;
  (e: "openTemplates"): void;
}>();

const title = ref(props.initialTemplate?.title || "");
const prompt = ref(props.initialTemplate?.prompt || "");
const assignedAgent = ref<string>(props.initialTemplate?.assigned_agent || "agy");
// 在已開啟的視窗內才選範本時，prop 會後來才變，需套用進欄位
watch(
  () => props.initialTemplate,
  (t) => {
    if (!t) return;
    title.value = t.title;
    prompt.value = t.prompt;
    assignedAgent.value = t.assigned_agent || "agy";
  },
);
const creator = ref<string>("claude");
const dependsOnTaskId = ref<string>("");
// 與後端 DEP_RESULT_PLACEHOLDER 一致
const DEP_RESULT = "{{dep.result}}";
const insertDepResult = () => {
  if (!prompt.value.includes(DEP_RESULT)) prompt.value = `${prompt.value.trimEnd()}\n\n前置任務的結果：\n${DEP_RESULT}`.trimStart();
};

const agentOptions = [
  { value: "agy", label: "@agy", hint: "本機 Autonomous Agent" },
  { value: "claude", label: "@claude", hint: "架構 / 派發 Agent" },
  { value: "none", label: "不限", hint: "任意 Agent 領取" },
];
const creatorOptions = ["claude", "human", "agy"];

const handleSubmit = () => {
  if (!title.value.trim() || !prompt.value.trim()) return;
  const rawDeps = dependsOnTaskId.value
    .split(/[,，\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  emit("create", {
    title: title.value.trim(),
    prompt: prompt.value.trim(),
    creator: creator.value.trim() || "human",
    assigned_agent: assignedAgent.value === "none" ? null : assignedAgent.value,
    depends_on_task_id: rawDeps[0] || null,
    dependency_ids: rawDeps,
  });
};

const field = "w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-ink placeholder:text-ink-3 focus:border-accent focus:outline-none focus:ring-3 focus:ring-accent-soft";
</script>

<template>
  <div
    class="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-4 pt-[12vh] backdrop-blur-[2px]"
    @click.self="emit('close')"
  >
    <form
      @submit.prevent="handleSubmit"
      @keydown.ctrl.enter="handleSubmit"
      @keydown.meta.enter="handleSubmit"
      class="w-full max-w-xl overflow-hidden rounded-xl border border-line bg-surface shadow-2xl"
    >
      <div class="flex items-center justify-between border-b border-line px-5 py-3.5">
        <div class="flex items-center gap-2">
          <h3 class="text-sm font-semibold text-ink">建立新任務</h3>
          <button
            type="button"
            @click="emit('openTemplates')"
            class="flex items-center gap-1 rounded-md bg-raised px-2 py-0.5 text-xs text-ink-2 hover:bg-raised/80 hover:text-ink transition-colors"
          >
            <span>📑</span>
            <span>選擇範本</span>
          </button>
        </div>
        <button
          type="button"
          @click="emit('close')"
          class="flex h-7 w-7 items-center justify-center rounded-md text-ink-3 hover:bg-raised hover:text-ink"
        >
          <Icon name="x" />
        </button>
      </div>

      <div class="flex flex-col gap-5 p-5">
        <input
          v-model="title"
          required
          autofocus
          placeholder="任務標題，例如：重構登入模組並撰寫單元測試"
          class="w-full bg-transparent text-lg font-semibold text-ink placeholder:text-ink-3 placeholder:font-normal focus:outline-none"
        />

        <textarea
          v-model="prompt"
          required
          rows="6"
          placeholder="詳細描述任務目標、驗證方式或輸入資料…"
          :class="[field, 'resize-none font-mono text-xs leading-relaxed']"
        ></textarea>

        <div>
          <span class="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-ink-3">指定 Agent</span>
          <div class="grid grid-cols-3 gap-2">
            <label
              v-for="opt in agentOptions"
              :key="opt.value"
              :class="[
                'cursor-pointer rounded-lg border px-3 py-2.5 transition-colors',
                assignedAgent === opt.value ? 'border-accent bg-accent-soft' : 'border-line hover:border-line-strong'
              ]"
            >
              <input v-model="assignedAgent" type="radio" :value="opt.value" class="sr-only" />
              <span class="flex items-center gap-1.5 text-xs font-semibold text-ink">
                <span :class="['h-1.5 w-1.5 rounded-full', agentMeta(opt.value).dot]"></span>
                {{ opt.label }}
              </span>
              <span class="mt-0.5 block text-[11px] text-ink-3">{{ opt.hint }}</span>
            </label>
          </div>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <div>
            <span class="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-ink-3">建立者</span>
            <div class="flex rounded-lg bg-raised p-0.5 text-xs">
              <button
                v-for="c in creatorOptions"
                :key="c"
                type="button"
                @click="creator = c"
                :class="[
                  'flex-1 rounded-md py-1.5 font-medium transition-colors',
                  creator === c ? 'bg-surface text-ink shadow-sm' : 'text-ink-2 hover:text-ink'
                ]"
              >
                {{ c }}
              </button>
            </div>
          </div>
          <div>
            <span class="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-ink-3">前置任務 ID（支援多個，以逗號分隔）</span>
            <input
              v-model="dependsOnTaskId"
              placeholder="id-1, id-2, …"
              :class="[field, 'py-1.5 font-mono text-xs']"
            />
            <p v-if="dependsOnTaskId.trim()" class="mt-1.5 text-[11px] leading-relaxed text-ink-3">
              Prompt 裡的
              <button type="button" @click="insertDepResult" class="font-mono text-accent hover:underline" title="插入到 Prompt 結尾">{{ DEP_RESULT }}</button>
              執行時會換成前置任務結果（亦可寫 <code class="font-mono text-[10px]">dep.&lt;短ID&gt;.result</code>）
            </p>
          </div>
        </div>
      </div>

      <div class="flex items-center justify-between border-t border-line bg-raised/50 px-5 py-3">
        <span class="text-[11px] text-ink-3"><kbd class="font-mono">Ctrl</kbd> + <kbd class="font-mono">Enter</kbd> 送出</span>
        <div class="flex gap-2">
          <button
            type="button"
            @click="emit('close')"
            class="rounded-lg px-3.5 py-1.5 text-xs font-medium text-ink-2 hover:bg-raised hover:text-ink"
          >
            取消
          </button>
          <button
            type="submit"
            :disabled="!title.trim() || !prompt.trim()"
            class="rounded-lg bg-accent px-3.5 py-1.5 text-xs font-semibold text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            建立任務
          </button>
        </div>
      </div>
    </form>
  </div>
</template>
