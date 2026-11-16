import { createRouter, createWebHashHistory } from "vue-router";
import HomePage from "./pages/HomePage.vue";
import AccessPage from "./pages/AccessPage.vue";
import ImagesPage from "./pages/ImagesPage.vue";
import UsagePage from "./pages/UsagePage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import ApiKeysPage from "./pages/ApiKeysPage.vue";

const scrollPositions = new Map<string, { left: number; top: number }>();
export const router = createRouter({
  history: createWebHashHistory(),
  scrollBehavior(to, _from, saved) {
    const content = document.querySelector<HTMLElement>(".main-shell");
    if (!content) return false;
    const previous = saved && scrollPositions.get(to.fullPath);
    const target = /^#client-(codex|claude)$/.test(to.hash)
      ? document.getElementById(to.hash.slice(1))
      : null;
    content.scrollTo(
      previous || {
        left: 0,
        top: target
          ? Math.max(
              0,
              target.getBoundingClientRect().top -
                content.getBoundingClientRect().top +
                content.scrollTop -
                16,
            )
          : 0,
      },
    );
    return false;
  },
  routes: [
    { path: "/", name: "home", component: HomePage },
    { path: "/images", name: "images", component: ImagesPage },
    { path: "/access", name: "access", component: AccessPage },
    { path: "/api-keys", name: "api-keys", component: ApiKeysPage },
    { path: "/usage", name: "usage", component: UsagePage },
    { path: "/settings", name: "settings", component: SettingsPage },
    { path: "/:pathMatch(.*)*", redirect: "/" },
  ],
});
router.beforeEach((_to, from) => {
  const content = document.querySelector<HTMLElement>(".main-shell");
  if (content)
    scrollPositions.set(from.fullPath, {
      left: content.scrollLeft,
      top: content.scrollTop,
    });
});
