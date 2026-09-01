import "@fontsource/source-sans-3/latin-400.css";
import "@fontsource/source-sans-3/latin-400-italic.css";
import "@fontsource/source-sans-3/latin-600.css";
import "@fontsource/source-sans-3/latin-700.css";
import "@fontsource/source-serif-4/latin-600.css";
import "@fontsource/source-serif-4/latin-700.css";
import "./styles.css";

import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { useClub } from "./stores/club";
import { useTheme } from "./stores/theme";

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);

useTheme().init();
// Brand (name, accent, timezone) arrives asynchronously; the shell paints with
// defaults so first render never waits on the network.
void useClub().load();

app.use(router).mount("#app");

if ("serviceWorker" in navigator && import.meta.env.PROD) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch(() => undefined);
  });
}
