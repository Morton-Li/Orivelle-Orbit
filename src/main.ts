import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import MenubarApp from "./MenubarApp.vue";
import { router } from "./router";
import "./style.css";

const menubar = window.location.hash === "#/menubar";
document.documentElement.classList.toggle("menubar-window", menubar);
const app = createApp(menubar ? MenubarApp : App).use(createPinia());
if (!menubar) app.use(router);
app.mount("#app");
