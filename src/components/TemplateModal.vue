<script setup lang="ts">
import { ref, onMounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { TaskTemplate } from "../types";
import { agentMeta } from "../ui";
import Icon from "./Icon.vue";

const props = defineProps<{
  templates: TaskTemplate[];
  fetchTemplates: () => Promise<void>;
  createTemplate: (tmpl: Partial<TaskTemplate>) => Promise<TaskTemplate>;
  deleteTemplate: (id: string) => Promise<void>;
  presetTemplate?: Partial<TaskTemplate> | null;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "select", tmpl: TaskTemplate): void;
}>();

const isCreating = ref(false);
const newTmpl = ref<Partial<TaskTemplate>>({
  name: "",
  description: "",
  title: "",
  prompt: "",
  assigned_agent: "agy",
  model: "",
  workdir: "",
  auto_approve: true,
  isolate: false,
  priority: 0,
});

onMounted(() => {
  props.fetchTemplates();
  if (props.presetTemplate) {
    newTmpl.value = { ...newTmpl.value, ...props.presetTemplate };
    isCreating.value = true;
  }
});

const chooseWorkdir = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "選擇範本預設工作目錄",
    });
    if (selected && typeof selected === "string") {
      newTmpl.value.workdir = selected;
    }
  } catch (err) {
    console.warn("無法開啟資料夾選擇彈窗:", err);
  }
};

const handleSave = async () => {
  if (!newTmpl.value.name?.trim() || !newTmpl.value.title?.trim() || !newTmpl.value.prompt?.trim()) return;
  await props.createTemplate({
    name: newTmpl.value.name.trim(),
    description: newTmpl.value.description?.trim() || undefined,
    title: newTmpl.value.title.trim(),
    prompt: newTmpl.value.prompt.trim(),
    assigned_agent: newTmpl.value.assigned_agent || undefined,
    model: newTmpl.value.model?.trim() || undefined,
    workdir: newTmpl.value.workdir?.trim() || undefined,
    auto_approve: newTmpl.value.auto_approve ?? true,
    isolate: newTmpl.value.isolate ?? false,
    priority: Number(newTmpl.value.priority) || 0,
  });
  isCreating.value = false;
  newTmpl.value = {
    name: "",
    description: "",
    title: "",
    prompt: "",
    assigned_agent: "agy",
    model: "",
    workdir: "",
    auto_approve: true,
    isolate: false,
    priority: 0,
  };
};

const handleDelete = async (id: string) => {
  if (confirm("確定要刪除此範本嗎？")) {
    await props.deleteTemplate(id);
  }
};

const field = "w-full rounded-lg border border-line bg-surface px-3 py-2 text-xs text-ink placeholder:text-ink-3 focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent-soft";
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-[2px]">
    <div class="flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
      <!-- Header -->
      <div class="flex items-center justify-between border-b border-line px-5 py-3.5">
        <div class="flex items-center gap-2">
          <span class="text-base">📑</span>
          <h3 class="text-sm font-semibold text-ink">任務範本庫 (Task Templates)</h3>
        </div>
        <div class="flex items-center gap-2">
          <button
            v-if="!isCreating"
            @click="isCreating = true"
            class="rounded-lg bg-accent px-3 py-1 text-xs font-semibold text-accent-ink hover:opacity-90 transition-opacity"
          >
            + 新增範本
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
      <div class="flex-1 overflow-y-auto p-5 text-xs text-ink">
        <!-- Create Form -->
        <div v-if="isCreating" class="mb-5 rounded-xl border border-line bg-surface-alt p-4">
          <div class="mb-3 flex items-center justify-between">
            <h4 class="text-xs font-bold text-ink">建立常用範本</h4>
            <button @click="isCreating = false" class="text-ink-3 hover:text-ink text-xs">取消</button>
          </div>
          <div class="flex flex-col gap-3">
            <div class="grid grid-cols-2 gap-3">
              <div>
                <label class="mb-1 block font-semibold text-ink-3">範本名稱 *</label>
                <input v-model="newTmpl.name" placeholder="例如：程式碼重構 + 測試" :class="field" />
              </div>
              <div>
                <label class="mb-1 block font-semibold text-ink-3">簡短說明</label>
                <input v-model="newTmpl.description" placeholder="用途簡述" :class="field" />
              </div>
            </div>

            <div>
              <label class="mb-1 block font-semibold text-ink-3">任務預設標題 *</label>
              <input v-model="newTmpl.title" placeholder="重構模組" :class="field" />
            </div>

            <div>
              <label class="mb-1 block font-semibold text-ink-3">預設 Prompt *</label>
              <textarea v-model="newTmpl.prompt" rows="3" placeholder="詳細指令…" :class="[field, 'resize-none font-mono']"></textarea>
            </div>

            <div class="grid grid-cols-3 gap-3">
              <div>
                <label class="mb-1 block font-semibold text-ink-3">指定 Agent</label>
                <select v-model="newTmpl.assigned_agent" :class="field">
                  <option value="agy">@agy</option>
                  <option value="claude">@claude</option>
                  <option value="">不限</option>
                </select>
              </div>
              <div>
                <label class="mb-1 block font-semibold text-ink-3">指定模型 (選填)</label>
                <input v-model="newTmpl.model" placeholder="預設" :class="field" />
              </div>
              <div>
                <div class="mb-1 flex items-center justify-between">
                  <label class="block font-semibold text-ink-3">工作目錄 (選填)</label>
                  <button type="button" @click="chooseWorkdir" class="text-[11px] text-accent hover:underline flex items-center gap-0.5">
                    <Icon name="folder" :size="11" />
                    <span>瀏覽</span>
                  </button>
                </div>
                <div class="relative flex items-center">
                  <input v-model="newTmpl.workdir" placeholder="/path/to/project" :class="[field, 'pr-7']" />
                  <button type="button" @click="chooseWorkdir" class="absolute right-2 text-ink-3 hover:text-ink">
                    <Icon name="folder" :size="12" />
                  </button>
                </div>
              </div>
            </div>

            <div class="flex items-center gap-6 pt-1">
              <label class="flex items-center gap-2 cursor-pointer text-ink">
                <input type="checkbox" v-model="newTmpl.auto_approve" class="rounded text-accent" />
                <span>免權限確認 (auto_approve)</span>
              </label>
              <label class="flex items-center gap-2 cursor-pointer text-ink">
                <input type="checkbox" v-model="newTmpl.isolate" class="rounded text-accent" />
                <span>獨立 Git Worktree 隔離</span>
              </label>
            </div>

            <div class="mt-2 flex justify-end gap-2">
              <button @click="isCreating = false" class="rounded-lg px-3 py-1.5 text-ink-2 hover:bg-raised">取消</button>
              <button
                @click="handleSave"
                :disabled="!newTmpl.name?.trim() || !newTmpl.title?.trim() || !newTmpl.prompt?.trim()"
                class="rounded-lg bg-accent px-4 py-1.5 font-semibold text-accent-ink hover:opacity-90 disabled:opacity-40"
              >
                儲存範本
              </button>
            </div>
          </div>
        </div>

        <!-- Template List -->
        <div v-if="templates.length === 0 && !isCreating" class="py-12 text-center text-ink-3">
          目前尚未建立任何範本，點擊右上角「+ 新增範本」來儲存常用的工作組合。
        </div>

        <div v-else class="flex flex-col gap-3">
          <div
            v-for="tmpl in templates"
            :key="tmpl.id"
            class="flex items-start justify-between rounded-xl border border-line bg-surface-alt p-4 transition-all hover:border-line-strong"
          >
            <div class="flex-1 pr-4">
              <div class="flex items-center gap-2">
                <h4 class="font-bold text-ink text-sm">{{ tmpl.name }}</h4>
                <span v-if="tmpl.assigned_agent" class="flex items-center gap-1 rounded bg-raised px-1.5 py-0.5 text-[10px] font-semibold text-ink">
                  <span :class="['h-1.5 w-1.5 rounded-full', agentMeta(tmpl.assigned_agent).dot]"></span>
                  @{{ tmpl.assigned_agent }}
                </span>
                <span v-if="tmpl.model" class="rounded bg-raised px-1.5 py-0.5 text-[10px] font-mono text-ink-2">
                  {{ tmpl.model }}
                </span>
                <span v-if="tmpl.isolate" class="rounded bg-purple-500/10 text-purple-600 dark:text-purple-400 px-1.5 py-0.5 text-[10px] font-medium">
                  worktree 隔離
                </span>
              </div>
              <p v-if="tmpl.description" class="mt-1 text-ink-3">{{ tmpl.description }}</p>
              <div class="mt-2 rounded bg-surface p-2 text-ink-2 font-mono text-[11px] truncate">
                <span class="text-ink font-semibold">{{ tmpl.title }}</span>: {{ tmpl.prompt }}
              </div>
            </div>

            <div class="flex items-center gap-2 shrink-0">
              <button
                @click="emit('select', tmpl)"
                class="rounded-lg bg-accent px-3 py-1.5 font-semibold text-accent-ink hover:opacity-90 transition-opacity"
              >
                帶入使用
              </button>
              <button
                @click="handleDelete(tmpl.id)"
                class="flex h-7 w-7 items-center justify-center rounded-lg text-ink-3 hover:bg-rose-500/10 hover:text-rose-500"
                title="刪除範本"
              >
                <Icon name="trash" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
