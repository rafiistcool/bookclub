import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import { i18n } from "./i18n";
import router from "./router";
import { useClub } from "./stores/club";
import { useLocale } from "./stores/locale";
import { useTheme } from "./stores/theme";
import "./styles/index.css";

const app = createApp(App);
app.use(createPinia());
app.use(i18n);

// The club config carries the accent Paper is built around, so it has to land
// before the theme store decides whether to apply that inline override.
await useClub().load();
useTheme().init();
useLocale().init();

app.use(router).mount("#app");
