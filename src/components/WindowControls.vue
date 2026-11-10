<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../shared/i18n";
import { useOrbitStore } from "../stores/orbit";
import OrbitIcon from "./OrbitIcon.vue";

const store = useOrbitStore();
const appWindow = getCurrentWindow();
const maximized = ref(false);
let unlisten: (() => void) | undefined;
let disposed = false;
async function updateMaximized() {
  maximized.value = await appWindow.isMaximized();
}
async function control(action: "minimize" | "toggleMaximize" | "close") {
  try {
    await appWindow[action]();
  } catch {
    store.notify(t("app.windowControlFailed"), "error");
  }
}
onMounted(async () => {
  try {
    const stop = await appWindow.onResized(
      () => void updateMaximized().catch(() => {}),
    );
    if (disposed) stop();
    else unlisten = stop;
    await updateMaximized();
  } catch {
    // Controls can still act on the window if its size cannot be observed.
  }
});
onUnmounted(() => {
  disposed = true;
  unlisten?.();
});
</script>

<template>
  <div class="window-controls">
    <button
      type="button"
      :title="t('app.minimize')"
      :aria-label="t('app.minimize')"
      @click="control('minimize')"
    >
      <OrbitIcon name="minimize" :size="16" />
    </button>
    <button
      type="button"
      :title="t(maximized ? 'app.restoreWindow' : 'app.maximize')"
      :aria-label="t(maximized ? 'app.restoreWindow' : 'app.maximize')"
      @click="control('toggleMaximize')"
    >
      <OrbitIcon :name="maximized ? 'unmaximize' : 'maximize'" :size="16" />
    </button>
    <button
      type="button"
      class="window-close"
      :title="t('action.close')"
      :aria-label="t('action.close')"
      @click="control('close')"
    >
      <OrbitIcon name="close" :size="16" />
    </button>
  </div>
</template>
