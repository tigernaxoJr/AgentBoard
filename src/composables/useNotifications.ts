import { ref, watch } from "vue";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

const STORAGE_KEY = "agent-board:notify";

const load = () => {
  try {
    return localStorage.getItem(STORAGE_KEY) !== "off";
  } catch {
    return true;
  }
};

// 全域共用一份開關，側邊欄與事件處理都讀這裡
export const notifyEnabled = ref(load());
watch(notifyEnabled, (on) => {
  try {
    localStorage.setItem(STORAGE_KEY, on ? "on" : "off");
  } catch {}
});

let permission: Promise<boolean> | null = null;
const ensurePermission = () =>
  (permission ??= isPermissionGranted()
    .then((ok) => ok || requestPermission().then((p) => p === "granted"))
    .catch(() => false));

/** 任務完成 / 失敗時發系統通知；視窗在前景時看板本身就看得到，不另外打擾 */
export async function notifyTask(title: string, body: string) {
  if (!notifyEnabled.value || document.hasFocus()) return;
  if (await ensurePermission()) sendNotification({ title, body });
}
