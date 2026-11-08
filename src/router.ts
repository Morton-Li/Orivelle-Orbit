import { createRouter, createWebHashHistory } from "vue-router";
import HomePage from "./pages/HomePage.vue";
import AccessPage from "./pages/AccessPage.vue";
import ImagesPage from "./pages/ImagesPage.vue";
import SettingsPage from "./pages/SettingsPage.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "home", component: HomePage },
    { path: "/images", name: "images", component: ImagesPage },
    { path: "/access", name: "access", component: AccessPage },
    { path: "/settings", name: "settings", component: SettingsPage },
    { path: "/:pathMatch(.*)*", redirect: "/" },
  ],
});
