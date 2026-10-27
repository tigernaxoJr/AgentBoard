import { createApp } from "vue";
import "./style.css";
import App from "./App.vue";
import { initTheme } from "./composables/useTheme";

initTheme();

createApp(App).mount("#app");
