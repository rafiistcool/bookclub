import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { useClub } from "./stores/club";
import "./styles/index.css";

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
await useClub().load();
app.use(router).mount("#app");
