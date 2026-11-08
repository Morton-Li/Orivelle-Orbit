import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  call,
  nativeAvailable,
  OrbitError,
  publicFailure,
  subscribeAuthorization,
  subscribeErrors,
  subscribeState,
} from "../shared/bridge";
import type {
  BootstrapView,
  AuthorizationEvent,
  ClientKind,
  ClientView,
  IpcCommands,
  ModelView,
  ScopedErrorEvent,
  SettingsView,
  TaskView,
} from "../shared/contracts";
import { locale, t } from "../shared/i18n";
import { configPolicyMessage } from "../shared/format";

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
    configuration_access: "unverified",
    configuration_access_revision: null,
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
    platform: null,
    pending_authorization: null,
    maintenance_active: false,
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
  const loginRequestId = ref<string | null>(null);
  let loginAttempt: {
    terminalEvents: Map<
      string,
      { event: AuthorizationEvent; handled: boolean }
    >;
  } | null = null;
  let unsubscribe: (() => void) | undefined;
  let unsubscribeErrors: (() => void) | undefined;
  let unsubscribeAuthorization: (() => void) | undefined;
  let modelGeneration = 0;
  let initPromise: Promise<void> | undefined;
  let settingsGeneration = 0;
  const account = computed(() =>
    ["authenticated", "refreshing"].includes(state.value.session.status)
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
      !busy.value.has("refresh-account") &&
      !state.value.maintenance_active &&
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
    const policyMessage =
      code === "panel_origin_not_allowed"
        ? t("login.officialOnly")
        : configPolicyMessage(code);
    notify(
      policyMessage ??
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
  function acceptAuthorization(event: AuthorizationEvent) {
    // Opening the browser can finish authorization before start_login returns.
    // Keep early events only for that in-flight command and match its returned ID.
    let terminal = loginAttempt?.terminalEvents.get(event.request_id);
    if (loginAttempt && !terminal) {
      terminal = { event, handled: false };
      loginAttempt.terminalEvents.set(event.request_id, terminal);
    }
    if (event.request_id !== loginRequestId.value) return;
    if (terminal) terminal.handled = true;
    loginPending.value = false;
    loginExpiresAt.value = null;
    loginRequestId.value = null;
    if (event.state === "connected") {
      loginOpen.value = false;
      notify(t("login.connected"));
    } else if (event.state === "denied") notify(t("login.denied"), "error");
    else if (event.state === "expired") notify(t("login.expired"), "error");
    else if (event.state === "failed") report(publicFailure(event.error));
    else notify(t("login.cancelled"));
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
    const previousAccountId = account.value?.id;
    state.value = next;
    initialized.value = true;
    locale.value = next.settings.locale;
    loginPending.value = next.pending_authorization !== null;
    loginExpiresAt.value = next.pending_authorization?.expires_at ?? null;
    if (next.pending_authorization)
      loginRequestId.value = next.pending_authorization.request_id;
    if (
      next.session.status === "authenticated" &&
      !next.pending_authorization &&
      account.value?.id !== previousAccountId
    )
      loginOpen.value = false;
    if (scope.value !== before) {
      modelGeneration++;
      models.value = [];
      void loadModels();
    }
    return true;
  }
  async function loadModels(): Promise<boolean> {
    const connection = activeConnection.value;
    const capturedScope = scope.value;
    const generation = ++modelGeneration;
    if (!connection || state.value.session.status !== "authenticated") {
      if (state.value.session.status !== "refreshing") models.value = [];
      modelsLoading.value = false;
      return false;
    }
    modelsLoading.value = true;
    try {
      const result = await call("list_models", {
        connection_id: connection.id,
      });
      if (generation !== modelGeneration || capturedScope !== scope.value)
        return false;
      models.value = result.filter(
        (model) => model.provider_id === connection.provider_id,
      );
      return true;
    } catch (error) {
      if (generation === modelGeneration && capturedScope === scope.value)
        report(error);
      return false;
    } finally {
      if (generation === modelGeneration) modelsLoading.value = false;
    }
  }
  async function perform<K extends keyof IpcCommands>(
    key: string,
    command: K,
    request: IpcCommands[K]["args"],
  ): Promise<IpcCommands[K]["result"] | undefined> {
    if (
      state.value.maintenance_active &&
      !["bootstrap", "list_models", "prepare_diagnostics"].includes(command)
    ) {
      notify(t("common.updateMaintenance"));
      return undefined;
    }
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
  async function refresh(remote = false) {
    if (busy.value.has("refresh")) return;
    busy.value.add("refresh");
    try {
      const result = await perform("bootstrap", "bootstrap", {});
      if (result) acceptState(result);
      const session = state.value.session;
      // A successful authorization can precede its first account response.
      // Recover that read without allowing paid actions or rendering another actor.
      const connection =
        activeConnection.value ??
        (session.status === "authenticated" && !session.account
          ? (state.value.connections.find(
              (candidate) =>
                candidate.id === state.value.active_connection_id &&
                candidate.panel_origin === session.panel_origin,
            ) ?? null)
          : null);
      if (
        remote &&
        connection &&
        state.value.session.status === "authenticated"
      ) {
        const captured = scope.value;
        const updated = await perform("refresh-account", "refresh_account", {
          connection_id: connection.id,
          expected_revision: connection.revision,
        });
        if (updated) {
          acceptState(updated);
          if (captured === scope.value) await loadModels();
        }
      }
    } finally {
      busy.value.delete("refresh");
    }
  }
  async function initialize(): Promise<void> {
    if (initPromise) return initPromise;
    initPromise = (async () => {
      if (!native.value) return;
      detecting.value = true;
      try {
        unsubscribe = await subscribeState(acceptState);
        unsubscribeErrors = await subscribeErrors(acceptError);
        unsubscribeAuthorization =
          await subscribeAuthorization(acceptAuthorization);
        await refresh(true);
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
    unsubscribeAuthorization?.();
    unsubscribeAuthorization = undefined;
  }
  async function login() {
    if (loginAttempt || loginPending.value) return undefined;
    const attempt = {
      terminalEvents: new Map<
        string,
        {
          event: AuthorizationEvent;
          handled: boolean;
        }
      >(),
    };
    loginAttempt = attempt;
    try {
      const result = await perform("login", "start_login", {});
      if (result) {
        const terminal = attempt.terminalEvents.get(result.request_id);
        if (!terminal?.handled) {
          loginRequestId.value = result.request_id;
          if (terminal) acceptAuthorization(terminal.event);
          else {
            loginPending.value = true;
            loginExpiresAt.value = result.expires_at;
          }
        }
        await refresh();
      }
      return result;
    } finally {
      loginAttempt = null;
    }
  }
  async function cancelLogin() {
    const id = loginRequestId.value;
    if (loginPending.value && id) {
      const result = await perform("login-cancel", "cancel_login", {
        request_id: id,
      });
      if (!result) return;
      acceptState(result);
    }
    loginOpen.value = false;
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
    loginRequestId,
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
    acceptAuthorization,
    loadModels,
    perform,
    refresh,
    initialize,
    dispose,
    login,
    cancelLogin,
    selectConnection,
    selectNode,
    saveSettings,
    acceptTask,
  };
});
