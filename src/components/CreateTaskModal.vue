<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { AgentDef, CreateTaskPayload, Task, TaskTemplate } from "../types";
import { agentMeta } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  initialTemplate?: TaskTemplate | null;
  tasks?: Task[];
  agents?: AgentDef[];
}>();

const emit = defineEmits<{
  (e: "create", payload: CreateTaskPayload): void;
  (e: "close"): void;
  (e: "openTemplates"): void;
}>();

const title = ref(props.initialTemplate?.title || "");
const prompt = ref(props.initialTemplate?.prompt || "");
const assignedAgent = ref<string>(props.initialTemplate?.assigned_agent || "agy");
const model = ref<string>(props.initialTemplate?.model || "");
const workdir = ref<string>(props.initialTemplate?.workdir || "");
const isolate = ref<boolean>(props.initialTemplate?.isolate || false);
const autoApprove = ref<boolean>(props.initialTemplate?.auto_approve ?? true);
const selectedDepIds = ref<string[]>([]);
const showDepPicker = ref(false);

const currentAgentDef = computed(() => props.agents?.find((a) => a.id === assignedAgent.value));

// 換 Agent 時，若原本的模型不在新 Agent 支援清單中則重設為預設
watch(assignedAgent, () => {
  if (model.value && !currentAgentDef.value?.models.includes(model.value)) {
    model.value = "";
  }
});

// 在已開啟的視窗內才選範本時，prop 會後來才變，需套用進欄位
watch(
  () => props.initialTemplate,
  (t) => {
    if (!t) return;
    title.value = t.title;
    prompt.value = t.prompt;
    assignedAgent.value = t.assigned_agent || "agy";
    model.value = t.model || "";
    workdir.value = t.workdir || "";
    isolate.value = t.isolate || false;
    autoApprove.value = t.auto_approve ?? true;
  },
);
const creator = ref<string>("claude");

// 與後端 DEP_RESULT_PLACEHOLDER 一致
const DEP_RESULT = "{{dep.result}}";
const insertDepResult = () => {
  if (!prompt.value.includes(DEP_RESULT)) prompt.value = `${prompt.value.trimEnd()}\n\n前置任務的結果：\n${DEP_RESULT}`.trimStart();
};

const toggleDep = (id: string) => {
  if (selectedDepIds.value.includes(id)) {
    selectedDepIds.value = selectedDepIds.value.filter((d) => d !== id);
  } else {
    selectedDepIds.value.push(id);
  }
};

const chooseWorkdir = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "選擇任務工作目錄",
    });
    if (selected && typeof selected === "string") {
      workdir.value = selected;
    }
  } catch (err) {
    console.warn("無法開啟資料夾選擇彈窗（可能在純瀏覽器模式）:", err);
  }
};

const agentOptions = [
  { value: "agy", label: "@agy", hint: "本機 Autonomous Agent" },
  { value: "claude", label: "@claude", hint: "架構 / 派發 Agent" },
  { value: "none", label: "不限", hint: "任意 Agent 領取" },
];
const creatorOptions = ["claude", "human", "agy"];

const handleSubmit = () => {
  if (!title.value.trim() || !prompt.value.trim()) return;
  emit("create", {
    title: title.value.trim(),
    prompt: prompt.value.trim(),
    creator: creator.value.trim() || "human",
    assigned_agent: assignedAgent.value === "none" ? null : assignedAgent.value,
    dependency_ids: selectedDepIds.value,
    workdir: workdir.value.trim() || null,
    model: model.value.trim() || null,
    isolate: isolate.value,
    auto_approve: autoApprove.value,
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
          <span class="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-ink-3">指定 Agent 與模型</span>
          <div class="grid grid-cols-3 gap-2 mb-3">
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

          <div v-if="currentAgentDef" class="rounded-lg border border-line bg-surface-alt p-3">
            <div class="flex items-center justify-between mb-1.5">
              <span class="text-[11px] font-semibold text-ink-3">指定模型（選填）</span>
              <span class="text-[11px] text-ink-3">預設：{{ currentAgentDef.default_model }}</span>
            </div>
            <select v-model="model" :class="[field, 'font-mono text-xs']">
              <option value="">預設模型（{{ currentAgentDef.default_model }}）</option>
              <option v-for="m in currentAgentDef.models" :key="m" :value="m">{{ m }}</option>
            </select>
          </div>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <div>
            <div class="mb-2 flex items-center justify-between">
              <span class="block text-[11px] font-semibold uppercase tracking-wider text-ink-3">工作目錄（選填）</span>
              <button
                type="button"
                @click="chooseWorkdir"
                class="flex items-center gap-1 text-[11px] font-medium text-accent hover:underline"
              >
                <Icon name="folder" :size="12" />
                <span>瀏覽目錄</span>
              </button>
            </div>
            <div class="relative flex items-center">
              <input
                v-model="workdir"
                placeholder="/home/user/my-project"
                :class="[field, 'py-1.5 font-mono text-xs pr-7']"
              />
              <button
                type="button"
                @click="chooseWorkdir"
                title="選擇目錄"
                class="absolute right-2 text-ink-3 hover:text-ink"
              >
                <Icon name="folder" :size="13" />
              </button>
            </div>
          </div>

          <div>
            <div class="mb-2 flex items-center justify-between">
              <span class="block text-[11px] font-semibold uppercase tracking-wider text-ink-3">
                前置任務（{{ selectedDepIds.length }} 個）
              </span>
              <button
                type="button"
                @click="showDepPicker = !showDepPicker"
                class="flex items-center gap-1 text-[11px] font-medium text-accent hover:underline"
              >
                <Icon name="link" :size="12" />
                <span>{{ showDepPicker ? "收起清單" : "挑選任務" }}</span>
              </button>
            </div>

            <!-- 已選取的前置 tags -->
            <div class="flex flex-wrap gap-1.5 min-h-[34px] rounded-lg border border-line bg-surface p-1.5">
              <span v-if="selectedDepIds.length === 0" class="text-xs text-ink-3 p-0.5">未設定前置任務</span>
              <span
                v-for="id in selectedDepIds"
                :key="id"
                class="inline-flex items-center gap-1 rounded bg-amber-500/10 px-2 py-0.5 font-mono text-[11px] text-amber-600 dark:text-amber-400"
              >
                <span>#{{ id.slice(0, 8) }}</span>
                <button type="button" @click="toggleDep(id)" class="hover:text-amber-800">×</button>
              </span>
            </div>

            <!-- 彈出/展開可選清單 -->
            <div v-if="showDepPicker" class="mt-2 max-h-44 overflow-y-auto rounded-lg border border-line bg-surface-alt p-2 flex flex-col gap-1">
              <div v-if="!tasks || tasks.length === 0" class="text-center text-xs text-ink-3 py-2">
                目前沒有任何已有任務可供挑選
              </div>
              <label
                v-for="t in tasks"
                :key="t.id"
                class="flex items-center gap-2 rounded px-2 py-1.5 hover:bg-raised text-xs cursor-pointer select-none"
              >
                <input
                  type="checkbox"
                  :checked="selectedDepIds.includes(t.id)"
                  @change="toggleDep(t.id)"
                  class="rounded text-accent"
                />
                <span class="font-mono text-ink-3">#{{ t.id.slice(0, 6) }}</span>
                <span class="font-medium text-ink truncate flex-1">{{ t.title }}</span>
                <span class="text-[10px] text-ink-3 uppercase">{{ t.status }}</span>
              </label>
            </div>
          </div>
        </div>

        <div class="flex items-center justify-between rounded-lg border border-line bg-surface-alt px-3 py-2 text-xs">
          <label class="flex items-center gap-2 cursor-pointer text-ink">
            <input type="checkbox" v-model="isolate" class="rounded text-accent" />
            <span>Git Worktree 隔離執行</span>
          </label>
          <label class="flex items-center gap-2 cursor-pointer text-ink">
            <input type="checkbox" v-model="autoApprove" class="rounded text-accent" />
            <span>免權限確認 (auto_approve)</span>
          </label>
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
          <div class="flex items-end pb-1">
            <p v-if="selectedDepIds.length > 0" class="text-[11px] leading-relaxed text-ink-3">
              Prompt 裡的
              <button type="button" @click="insertDepResult" class="font-mono text-accent hover:underline" title="插入到 Prompt 結尾">{{ DEP_RESULT }}</button>
              執行時會換成前置任務結果
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
