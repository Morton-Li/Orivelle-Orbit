import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  call,
  nativeAvailable,
  subscribeState,
  subscribeMenubarOpened,
  subscribeMenubarHidden,
} from "../shared/bridge";
import { emptyState } from "../stores/orbit";
import type {
  BootstrapView,
  MenubarDestination,
  MenubarRequest,
} from "../shared/contracts";
import { dateRange, type UsageSummary } from "../shared/usage";
import { locale, t } from "../shared/i18n";
import { useAppearance } from "./useAppearance";

/** Read-only projection; no startup update checks or client configuration. */
export function useMenubar() {
  const state = ref(emptyState());
  const initialized = ref(false);
  const visible = ref(false);
  const refreshing = ref(false);
  const summary = ref<UsageSummary | null>(null);
  const usageFailed = ref(false);
  const notice = ref("");
  const now = ref(Date.now());
  const stops: (() => void)[] = [];
  let disposed = false;
  let generation = 0;
  let usageUpdated = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  useAppearance(computed(() => state.value.settings));
  const account = computed(() =>
    ["authenticated", "refreshing"].includes(state.value.session.status)
      ? state.value.session.account
      : null,
  );
  const connection = computed(
    () =>
      state.value.connections.find(
        (c) =>
          c.id === state.value.active_connection_id &&
          c.account_id === account.value?.id &&
          c.panel_origin === state.value.session.panel_origin,
      ) ?? null,
  );
  const scope = computed(() =>
    connection.value
      ? `${connection.value.panel_origin}/${connection.value.account_id}/${connection.value.id}/${connection.value.revision}`
      : "none",
  );
  const tasks = computed(() =>
    state.value.tasks.filter(
      (task) =>
        task.account_id === account.value?.id &&
        state.value.connections.some(
          (c) =>
            c.id === task.connection_id &&
            c.account_id === task.account_id &&
            c.panel_origin === connection.value?.panel_origin,
        ),
    ),
  );
  const activeTasks = computed(() =>
    tasks.value.filter((task) =>
      ["queued", "sending", "waiting", "saving"].includes(task.state),
    ),
  );
  const attentionTask = computed(
    () =>
      activeTasks.value[0] ??
      tasks.value.find((task) =>
        ["save_failed", "unconfirmed"].includes(task.state),
      ) ??
      tasks.value.find((task) => task.state === "completed") ??
      null,
  );

  function acceptState(next: BootstrapView) {
    if (
      disposed ||
      next.schema_version !== 1 ||
      !Number.isSafeInteger(next.state_revision) ||
      next.state_revision < 0 ||
      (initialized.value && next.state_revision <= state.value.state_revision)
    )
      return;
    state.value = next;
    initialized.value = true;
    locale.value = next.settings.locale;
  }
  async function loadUsage() {
    const current = connection.value;
    if (
      !current ||
      state.value.session.status !== "authenticated" ||
      state.value.maintenance_active ||
      !visible.value
    )
      return;
    const owner = scope.value;
    const token = ++generation;
    const range = dateRange(1);
    usageUpdated = Date.now();
    if (summary.value && summary.value.range.date_from !== range.date_from)
      summary.value = null;
    try {
      const value = await call("read_usage_summary", {
        connection_id: current.id,
        expected_revision: current.revision,
        ...range,
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
      });
      if (
        !disposed &&
        token === generation &&
        owner === scope.value &&
        visible.value
      ) {
        summary.value = value;
        usageFailed.value = false;
      }
    } catch {
      if (!disposed && token === generation && owner === scope.value)
        usageFailed.value = true;
    }
  }
  watch(
    scope,
    () => {
      generation++;
      summary.value = null;
      usageFailed.value = false;
      notice.value = "";
      if (visible.value) void loadUsage();
    },
    { flush: "sync" },
  );
  async function refresh(remote = true) {
    if (refreshing.value || disposed) return;
    refreshing.value = true;
    notice.value = "";
    try {
      acceptState(await call("bootstrap", {}));
      const current = connection.value;
      if (
        remote &&
        current &&
        state.value.session.status === "authenticated" &&
        !state.value.maintenance_active
      ) {
        acceptState(
          await call("refresh_account", {
            connection_id: current.id,
            expected_revision: current.revision,
          }),
        );
      }
      await loadUsage();
    } catch {
      if (!disposed) notice.value = t("menubar.refreshFailed");
    } finally {
      refreshing.value = false;
    }
  }
  function shown() {
    visible.value = true;
    now.value = Date.now();
    if (timer) clearInterval(timer);
    timer = setInterval(() => {
      now.value = Date.now();
      if (now.value - usageUpdated >= 60_000) void loadUsage();
    }, 1000);
    void refresh();
  }
  function hidden() {
    visible.value = false;
    generation++;
    if (timer) clearInterval(timer);
    timer = undefined;
  }
  function visibilityChanged() {
    if (document.visibilityState === "hidden") hidden();
  }
  async function control(request: MenubarRequest) {
    try {
      await call("menubar_control", request);
    } catch {
      notice.value = t("app.windowControlFailed");
    }
  }
  const open = (destination: MenubarDestination) =>
    control({ action: "open", destination });
  onMounted(async () => {
    document.addEventListener("visibilitychange", visibilityChanged);
    if (!nativeAvailable()) return;
    try {
      for (const subscribe of [
        () => subscribeState(acceptState),
        () => subscribeMenubarOpened(shown),
        () => subscribeMenubarHidden(hidden),
      ]) {
        const stop = await subscribe();
        if (disposed) {
          stop();
          return;
        }
        stops.push(stop);
      }
      await refresh(false);
      if (!disposed) await control({ action: "ready" });
    } catch {
      if (!disposed) notice.value = t("menubar.refreshFailed");
    }
  });
  onUnmounted(() => {
    disposed = true;
    generation++;
    stops.forEach((stop) => stop());
    if (timer) clearInterval(timer);
    document.removeEventListener("visibilitychange", visibilityChanged);
  });
  return {
    state,
    initialized,
    account,
    connection,
    tasks,
    activeTasks,
    attentionTask,
    summary,
    usageFailed,
    notice,
    now,
    refreshing,
    refresh,
    open,
    control,
  };
}
