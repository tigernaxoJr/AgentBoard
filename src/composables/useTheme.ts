import { ref } from "vue";

export type ThemePref = "system" | "light" | "dark";
const KEY = "agent_board_theme";
const media = window.matchMedia("(prefers-color-scheme: dark)");

const read = (): ThemePref => {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "light" || v === "dark") return v;
  } catch {}
  return "system";
};

export const themePref = ref<ThemePref>(read());

const apply = () => {
  const dark = themePref.value === "dark" || (themePref.value === "system" && media.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
};

export const initTheme = () => {
  apply();
  media.addEventListener("change", apply);
};

export const cycleTheme = () => {
  const order: ThemePref[] = ["system", "light", "dark"];
  themePref.value = order[(order.indexOf(themePref.value) + 1) % order.length];
  try {
    if (themePref.value === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, themePref.value);
  } catch {}
  apply();
};
