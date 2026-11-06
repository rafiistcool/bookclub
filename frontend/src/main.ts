import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import { installAssetRecovery } from "./assetRecovery";
import { i18n } from "./i18n";
import router from "./router";
import { useService } from "./stores/service";
import { useLocale } from "./stores/locale";
import { useTheme } from "./stores/theme";
// The faces tokens.css names, bundled (no third-party request). Each file
// carries every subset with its unicode-range, so a browser fetches only the
// subsets a page uses.
import "@fontsource/source-sans-3/400.css";
import "@fontsource/source-sans-3/400-italic.css";
import "@fontsource/source-sans-3/600.css";
import "@fontsource/source-sans-3/700.css";
import "@fontsource/source-serif-4/400.css";
import "@fontsource/source-serif-4/400-italic.css";
import "@fontsource/source-serif-4/600.css";
import "@fontsource/source-serif-4/700.css";
import "./styles/index.css";

installAssetRecovery();
const app = createApp(App);
app.use(createPinia());
app.use(i18n);

// The club config carries the accent Paper is built around, so it has to land
// before the theme store decides whether to apply that inline override.
await useService().load();
useTheme().init();
useLocale().init();

app.use(router).mount("#app");
