<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { useOrbitStore } from "./stores/orbit";
import { locale, t } from "./shared/i18n";
import OrbitIcon from "./components/OrbitIcon.vue";
import OrbitDialog from "./components/OrbitDialog.vue";
const store = useOrbitStore();
const router = useRouter();
const systemDark = ref(false);
const dark = computed(
  () =>
    store.state.settings.theme === "dark" ||
    (store.state.settings.theme === "system" && systemDark.value),
);
const navigation = [
  { path: "/", key: "nav.home", icon: "home" },
  { path: "/images", key: "nav.images", icon: "images" },
  { path: "/access", key: "nav.access", icon: "access" },
  { path: "/settings", key: "nav.settings", icon: "settings" },
] as const;
let media: MediaQueryList | undefined;
function updateSystem(event: MediaQueryListEvent) {
  systemDark.value = event.matches;
}
function keyboard(event: KeyboardEvent) {
  if (
    (!event.metaKey && !event.ctrlKey) ||
    event.altKey ||
    event.shiftKey ||
    event.repeat ||
    store.loginOpen ||
    document.querySelector('[role="dialog"]')
  )
    return;
  const route = navigation[Number(event.key) - 1];
  if (route && /^[1-4]$/.test(event.key)) {
    event.preventDefault();
    void router.push(route.path);
  }
}
async function login() {
  await store.login();
}
watch(
  dark,
  (value) => document.documentElement.classList.toggle("dark", value),
  { immediate: true },
);
watch(
  locale,
  (value) => {
    document.documentElement.lang = value;
  },
  { immediate: true },
);
onMounted(() => {
  media = window.matchMedia("(prefers-color-scheme: dark)");
  systemDark.value = media.matches;
  media.addEventListener("change", updateSystem);
  window.addEventListener("keydown", keyboard);
  void store.initialize();
});
onUnmounted(() => {
  media?.removeEventListener("change", updateSystem);
  window.removeEventListener("keydown", keyboard);
  store.dispose();
});
</script>
<template>
  <div class="app-shell">
    <aside class="sidebar">
      <RouterLink to="/" class="brand" aria-label="Orivelle Orbit"
        ><span class="brand-mark" aria-hidden="true"><span /></span
        ><span>Orivelle <strong>Orbit</strong></span></RouterLink
      >
      <nav :aria-label="t('nav.label')">
        <RouterLink
          v-for="item in navigation"
          :key="item.path"
          :to="item.path"
          class="nav-link"
          :class="{ 'nav-active': $route.path === item.path }"
          ><OrbitIcon :name="item.icon" /><span>{{
            t(item.key)
          }}</span></RouterLink
        >
      </nav>
      <div class="sidebar-bottom">
        <span
          class="connection-dot"
          :class="{ connected: store.state.session.status === 'authenticated' }"
        /><span class="truncate">{{
          store.account?.label ?? t("home.loggedOut")
        }}</span
        ><button
          class="icon-button"
          :aria-label="t('action.login')"
          @click="store.loginOpen = true"
        >
          <OrbitIcon name="plus" />
        </button>
      </div>
    </aside>
    <div class="main-shell">
      <header class="topbar">
        <span class="topbar-subtitle">{{ t("app.subtitle") }}</span
        ><span class="topbar-account truncate">{{
          store.activeConnection?.node_label ?? "Orivelle Orbit"
        }}</span
        ><button
          class="icon-button"
          :aria-label="t('action.refresh')"
          :disabled="!store.native || store.busy.has('refresh')"
          @click="store.refresh(true)"
        >
          <OrbitIcon name="refresh" />
        </button>
      </header>
      <div
        v-if="store.state.source === 'synthetic'"
        class="context-banner"
        role="status"
      >
        {{ t("app.synthetic") }}
      </div>
      <div v-if="!store.native" class="context-banner">
        {{ t("common.desktopRequired") }}
      </div>
      <main id="main-content" class="page-container" tabindex="-1">
        <p v-if="store.state.maintenance_active" class="info-box" role="status">
          {{ t("common.updateMaintenance") }}
        </p>
        <RouterView />
      </main>
    </div>
    <div
      v-if="store.notice"
      class="toast"
      :class="`toast-${store.notice.tone}`"
      :role="store.notice.tone === 'error' ? 'alert' : 'status'"
    >
      <span>{{ store.notice.text }}</span
      ><button
        class="icon-button"
        :aria-label="t('action.close')"
        @click="store.clearNotice"
      >
        <OrbitIcon name="close" />
      </button>
    </div>
    <OrbitDialog
      v-model:open="store.loginOpen"
      :title="t('login.title')"
      :description="t('login.description')"
    >
      <form class="dialog-form" @submit.prevent="login">
        <div>
          <p class="field-label">{{ t("login.service") }}</p>
          <p class="mono">https://orivelle.powerbyte.io</p>
          <p class="field-hint">{{ t("login.hint") }}</p>
        </div>
        <p v-if="store.loginPending" class="info-box" role="status">
          {{ t("login.waiting") }}
        </p>
        <div class="dialog-actions">
          <button
            type="button"
            class="button secondary"
            :disabled="store.busy.has('login-cancel')"
            @click="store.cancelLogin"
          >
            {{ t("action.cancel") }}</button
          ><button
            type="submit"
            class="button primary"
            :disabled="
              !store.native || store.busy.has('login') || store.loginPending
            "
          >
            <OrbitIcon name="external" />{{ t("action.continue") }}
          </button>
        </div>
      </form>
    </OrbitDialog>
  </div>
</template>
