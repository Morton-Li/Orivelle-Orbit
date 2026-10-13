import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  call,
  nativeAvailable,
  OrbitError,
  publicFailure,
  subscribeErrors,
  subscribeState,
} from "../shared/bridge";
import type {
  BootstrapView,
  ClientKind,
  ClientView,
  IpcCommands,
  ModelView,
  ScopedErrorEvent,
  SettingsView,
  TaskView,
} from "../shared/contracts";
import { locale, t } from "../shared/i18n";

const defaultSettings: SettingsView = {
  theme: "system",
  locale: "zh-CN",
  close_behavior: "platform_default",
  image_disk_limit_bytes: 2 * 1024 ** 3,
  preview_cache_limit_bytes: 128 * 1024 ** 2,
  backup_retention_days: 30,
  backup_max_groups: 5,
  check_updates_on_start: false,
};
function undetectedClient(client: ClientKind): ClientView {
  return {
    client,
    installed: false,
    display_name: client === "codex" ? "Codex" : "Claude Desktop",
    version: null,
    bundle_id: null,
    app_path: null,
    config_paths: [],
    platform_supported: false,
    managed: "unknown",
    configuration_state: "not_configured",
    loaded_state: "unconfirmed",
    connection_state: "not_tested",
    configuration_revision: null,
    loaded_evidence: null,
    connection_evidence: null,
    manual_steps: [],
  };
}
export function emptyState(): BootstrapView {
  return {
    schema_version: 1,
    state_revision: 0,
    source: "local",
    session: {
      status: "logged_out",
      panel_origin: null,
      account: null,
      inference_credential_status: "unavailable",
    },
    connections: [],
    installations: [],
    nodes: [],
    active_connection_id: null,
    clients: [undetectedClient("codex"), undetectedClient("claude")],
    settings: { ...defaultSettings },
    tasks: [],
    image_history: [],
    backups: [],
  };
}

export const useOrbitStore = defineStore("orbit", () => {
  const state = ref<BootstrapView>(emptyState());
  const initialized = ref(false);
  const detecting = ref(false);
  const native = ref(nativeAvailable());
  const models = ref<ModelView[]>([]);
  const modelsLoading = ref(false);
  const busy = ref(new Set<string>());
  const notice = ref<{
    text: string;
    tone: "success" | "error" | "neutral";
  } | null>(null);
  const loginOpen = ref(false);
  const loginPending = ref(false);
  const loginExpiresAt = ref<string | null>(null);
  let unsubscribe: (() => void) | undefined;
  let unsubscribeErrors: (() => void) | undefined;
  let modelGeneration = 0;
  let initPromise: Promise<void> | undefined;
  let settingsGeneration = 0;
  const account = computed(() =>
    state.value.session.status === "authenticated"
      ? state.value.session.account
      : null,
  );
  const activeConnection = computed(
    () =>
      state.value.connections.find(
        (connection) =>
          connection.id === state.value.active_connection_id &&
          connection.account_id === account.value?.id,
      ) ?? null,
  );
  const scope = computed(() =>
    activeConnection.value
      ? `${activeConnection.value.account_id}/${activeConnection.value.id}/${activeConnection.value.revision}/${activeConnection.value.credential_revision ?? ""}/${activeConnection.value.node_id ?? ""}`
      : "none",
  );
  const ready = computed(
    () =>
      state.value.session.status === "authenticated" &&
      state.value.session.inference_credential_status === "ready" &&
      activeConnection.value?.permission_state === "allowed" &&
      !!activeConnection.value.node_id,
  );
  // The native layer enforces ownership too. Rendering is restricted again so stale results cannot appear in another account.
  const tasks = computed(() =>
    state.value.tasks.filter(
      (task) =>
        task.account_id === account.value?.id &&
        state.value.connections.some(
          (connection) =>
            connection.id === task.connection_id &&
            connection.account_id === task.account_id &&
            connection.panel_origin === activeConnection.value?.panel_origin,
        ),
    ),
  );
  const history = computed(() =>
    state.value.image_history.filter(
      (image) =>
        image.account_id === account.value?.id &&
        tasks.value.some((task) => task.id === image.task_id),
    ),
  );
  const backups = computed(() =>
    state.value.backups.filter(
      (backup) =>
        backup.account_id === account.value?.id &&
        backup.panel_origin === activeConnection.value?.panel_origin,
    ),
  );
  const textModels = computed(() =>
    models.value.filter(
      (model) =>
        model.category === "text" && model.support_status !== "unsupported",
    ),
  );
  const imageModels = computed(() =>
    models.value.filter(
      (model) =>
        model.category === "image" && model.support_status !== "unsupported",
    ),
  );

  function notify(
    text: string,
    tone: "success" | "error" | "neutral" = "neutral",
  ) {
    notice.value = { text, tone };
  }
  function clearNotice() {
    notice.value = null;
  }
  function report(error: unknown) {
    const code = error instanceof OrbitError ? error.code : "operation_failed";
    notify(
      `${error instanceof OrbitError ? error.message : t("common.failed")} (${code})`,
      "error",
    );
  }
  function acceptError(event: ScopedErrorEvent) {
    const global =
      event.account_id === null &&
      event.connection_id === null &&
      event.connection_revision === null;
    const connection = activeConnection.value;
    if (
      !global &&
      (!connection ||
        event.account_id !== account.value?.id ||
        event.connection_id !== connection.id ||
        event.connection_revision !== connection.revision)
    )
      return;
    report(publicFailure(event.error));
  }
  function acceptState(next: BootstrapView): boolean {
    if (
      next.schema_version !== 1 ||
      !Number.isSafeInteger(next.state_revision) ||
      next.state_revision < 0 ||
      (initialized.value && next.state_revision <= state.value.state_revision)
    )
      return false;
    const before = scope.value;
    state.value = next;
    initialized.value = true;
    locale.value = next.settings.locale;
    if (next.session.status !== "authorizing") {
      loginPending.value = false;
      loginExpiresAt.value = null;
    }
    if (next.session.status === "authenticated") loginOpen.value = false;
    if (scope.value !== before) {
      modelGeneration++;
      models.value = [];
      void loadModels();
    }
    return true;
  }
  async function loadModels() {
    const connection = activeConnection.value;
    const capturedScope = scope.value;
    const generation = ++modelGeneration;
    models.value = [];
    if (!connection || state.value.session.status !== "authenticated") {
      modelsLoading.value = false;
      return;
    }
    modelsLoading.value = true;
    try {
      const result = await call("list_models", {
        connection_id: connection.id,
      });
      if (generation === modelGeneration && capturedScope === scope.value)
        models.value = result.filter(
          (model) => model.provider_id === connection.provider_id,
        );
    } catch (error) {
      if (generation === modelGeneration && capturedScope === scope.value)
        report(error);
    } finally {
      if (generation === modelGeneration) modelsLoading.value = false;
    }
  }
  async function perform<K extends keyof IpcCommands>(
    key: string,
    command: K,
    request: IpcCommands[K]["args"],
  ): Promise<IpcCommands[K]["result"] | undefined> {
    if (busy.value.has(key)) return undefined;
    busy.value.add(key);
    try {
      return await call(command, request);
    } catch (error) {
      report(error);
      return undefined;
    } finally {
      busy.value.delete(key);
    }
  }
  async function refresh() {
    const result = await perform("bootstrap", "bootstrap", {});
    if (result) acceptState(result);
  }
  async function initialize(): Promise<void> {
    if (initPromise) return initPromise;
    initPromise = (async () => {
      if (!native.value) return;
      detecting.value = true;
      try {
        unsubscribe = await subscribeState(acceptState);
        unsubscribeErrors = await subscribeErrors(acceptError);
        await refresh();
      } catch (error) {
        report(error);
      } finally {
        detecting.value = false;
      }
    })();
    return initPromise;
  }
  function dispose() {
    unsubscribe?.();
    unsubscribe = undefined;
    unsubscribeErrors?.();
    unsubscribeErrors = undefined;
  }
  async function login(panelOrigin: string) {
    const result = await perform("login", "start_login", {
      panel_origin: panelOrigin,
    });
    if (result) {
      loginPending.value = true;
      loginExpiresAt.value = result.expires_at;
      await refresh();
    }
    return result;
  }
  async function selectConnection(id: string) {
    const result = await perform("connection", "select_connection", {
      connection_id: id,
    });
    if (result) acceptState(result);
  }
  async function selectNode(id: string) {
    const connection = activeConnection.value;
    if (!connection) return;
    const result = await perform("node", "select_node", {
      connection_id: connection.id,
      node_id: id,
      expected_revision: connection.revision,
    });
    if (result) acceptState(result);
  }
  async function saveSettings(patch: Partial<SettingsView>) {
    if (busy.value.has("settings")) return;
    const generation = ++settingsGeneration;
    const result = await perform("settings", "update_settings", { patch });
    if (result && generation === settingsGeneration) {
      state.value = { ...state.value, settings: result };
      locale.value = result.locale;
    }
    return result;
  }
  function acceptTask(task: TaskView) {
    if (
      task.account_id !== account.value?.id ||
      !state.value.connections.some(
        (connection) =>
          connection.id === task.connection_id &&
          connection.account_id === task.account_id &&
          connection.panel_origin === activeConnection.value?.panel_origin,
      )
    )
      return;
    const index = state.value.tasks.findIndex((item) => item.id === task.id);
    // Command acknowledgements may arrive after a newer authoritative state event.
    // Existing tasks are refreshed through monotonic bootstrap/events, never replaced by acknowledgements.
    if (index >= 0) return;
    else state.value.tasks.unshift(task);
  }
  return {
    state,
    initialized,
    detecting,
    native,
    models,
    modelsLoading,
    busy,
    notice,
    loginOpen,
    loginPending,
    loginExpiresAt,
    account,
    activeConnection,
    scope,
    ready,
    tasks,
    history,
    backups,
    textModels,
    imageModels,
    notify,
    clearNotice,
    acceptState,
    acceptError,
    loadModels,
    perform,
    refresh,
    initialize,
    dispose,
    login,
    selectConnection,
    selectNode,
    saveSettings,
    acceptTask,
  };
});
