import { onMounted, onUnmounted, ref, watch, type Ref } from "vue";
import type { SettingsView } from "../shared/contracts";
import { locale } from "../shared/i18n";

/** Both windows follow Orbit's stored preference and the OS appearance. */
export function useAppearance(settings: Ref<SettingsView>) {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const systemDark = ref(media.matches);
  const updateSystem = (event: MediaQueryListEvent) => {
    systemDark.value = event.matches;
  };
  watch(
    () =>
      settings.value.theme === "dark" ||
      (settings.value.theme === "system" && systemDark.value),
    (dark) => document.documentElement.classList.toggle("dark", dark),
    { immediate: true },
  );
  watch(locale, (value) => (document.documentElement.lang = value), {
    immediate: true,
  });
  onMounted(() => media.addEventListener("change", updateSystem));
  onUnmounted(() => media.removeEventListener("change", updateSystem));
}
