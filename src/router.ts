import { createRouter, createWebHashHistory } from "vue-router";
import HomePage from "./pages/HomePage.vue";
import AccessPage from "./pages/AccessPage.vue";
import ImagesPage from "./pages/ImagesPage.vue";
import UsagePage from "./pages/UsagePage.vue";
import SettingsPage from "./pages/SettingsPage.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  scrollBehavior(to, _from, saved) {
    return (
      saved ??
      (/^#client-(codex|claude)$/.test(to.hash)
        ? { el: to.hash, top: 66 }
        : { top: 0 })
    );
  },
  routes: [
    { path: "/", name: "home", component: HomePage },
    { path: "/images", name: "images", component: ImagesPage },
    { path: "/access", name: "access", component: AccessPage },
    { path: "/usage", name: "usage", component: UsagePage },
    { path: "/settings", name: "settings", component: SettingsPage },
    { path: "/:pathMatch(.*)*", redirect: "/" },
  ],
});
