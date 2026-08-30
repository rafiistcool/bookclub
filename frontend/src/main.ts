import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { useClub } from "./stores/club";
import { useTheme } from "./stores/theme";
import "./styles/index.css";

const app = createApp(App);
app.use(createPinia());

// The club config carries the accent Paper is built around, so it has to land
// before the theme store decides whether to apply that inline override.
await useClub().load();
useTheme().init();

app.use(router).mount("#app");
