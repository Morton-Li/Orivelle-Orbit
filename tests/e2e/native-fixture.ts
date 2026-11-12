import type { Page } from "@playwright/test";
import type {
  AuthorizationEvent,
  BootstrapView,
} from "../../src/shared/contracts";
import {
  bootstrap,
  imageModel,
  syntheticClaudeConfigurationTarget,
  syntheticCodexCatalog,
  syntheticCodexConfigurationTarget,
  syntheticCodexModel,
  task,
} from "../fixtures";

export type FixtureControl = {
  counts: Record<string, number>;
  lastRequests: Record<string, Record<string, unknown>>;
  pushState: (state: BootstrapView) => void;
  pushAuthorization: (event: AuthorizationEvent) => void;
};
export async function fixture(
  page: Page,
  options: {
    state?: BootstrapView;
    paidFailure?: boolean;
    claudeImport?: boolean;
    restoreMode?: "blocked" | "recoverable";
  } = {},
) {
  const state = options.state ?? bootstrap();
  await page.addInitScript(
    ({
      initial,
      models,
      initialTask,
      paidFailure,
      claudeImport,
      restoreMode,
      codexCatalog,
      codexConfigurationTarget,
      codexRuntime,
      claudeConfigurationTarget,
    }) => {
      // Test-only in-memory IPC. This is injected by Playwright and is never included in the application bundle.
      const testWindow = window as unknown as {
        isTauri: boolean;
        __TAURI_INTERNALS__: {
          metadata: { currentWindow: { label: string } };
          invoke: (
            command: string,
            args?: Record<string, unknown>,
          ) => Promise<unknown>;
          transformCallback: (callback: (event: unknown) => void) => number;
        };
        __TAURI_EVENT_PLUGIN_INTERNALS__: {
          unregisterListener: (event: string, id: number) => void;
        };
        __ORBIT_TEST__: FixtureControl;
      };
      testWindow.isTauri = true;
      let current = initial;
      const callbacks = new Map<number, (event: unknown) => void>();
      const listeners = new Map<string, Set<number>>();
      let next = 0;
      function emit(event: string, payload: unknown) {
        for (const id of listeners.get(event) ?? [])
          callbacks.get(id)?.({ event, id, payload });
      }
      testWindow.__ORBIT_TEST__ = {
        counts: {},
        lastRequests: {},
        pushState(value) {
          current = value;
          emit("orbit://state", value);
        },
        pushAuthorization(value) {
          if (current.pending_authorization?.request_id === value.request_id) {
            current.pending_authorization = null;
            update();
          }
          emit("orbit://authorization", value);
        },
      };
      function update() {
        current.state_revision++;
        emit("orbit://state", structuredClone(current));
      }
      testWindow.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener(event, id) {
          listeners.get(event)?.delete(id);
          callbacks.delete(id);
        },
      };
      testWindow.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback(callback) {
          const id = ++next;
          callbacks.set(id, callback);
          return id;
        },
        async invoke(command, args = {}) {
          if (command === "plugin:window|is_maximized") return false;
          if (command === "plugin:event|listen") {
            const name = String(args.event);
            if (!listeners.has(name)) listeners.set(name, new Set());
            listeners.get(name)!.add(Number(args.handler));
            return args.handler;
          }
          if (command === "plugin:event|unlisten") return;
          const control = testWindow.__ORBIT_TEST__;
          control.counts[command] = (control.counts[command] ?? 0) + 1;
          const request = (args.request ?? {}) as Record<string, unknown>;
          control.lastRequests[command] = structuredClone(request);
          if (command === "bootstrap") return structuredClone(current);
          if (command === "refresh_account") return structuredClone(current);
          if (command === "start_login") {
            current.pending_authorization = {
              request_id: "synthetic-login",
              expires_at: new Date(Date.now() + 60000).toISOString(),
            };
            update();
            return current.pending_authorization;
          }
          if (command === "cancel_login") {
            testWindow.__ORBIT_TEST__.pushAuthorization({
              request_id: String(request.request_id),
              state: "cancelled",
              error: null,
            });
            return structuredClone(current);
          }
          if (command === "list_models") return models;
          if (command === "probe_nodes")
            return current.nodes
              .filter((node) =>
                (request.node_ids as string[]).includes(node.id),
              )
              .map((node, index) => ({
                node_id: node.id,
                node_revision: node.revision,
                status: "available",
                milliseconds: 82 + index * 50,
                measured_at: new Date().toISOString(),
              }));
          if (command === "cancel_node_probes") return null;
          if (command === "update_settings") {
            current.settings = {
              ...current.settings,
              ...(request.patch as object),
            };
            update();
            return current.settings;
          }
          if (command === "create_image_task") {
            const created = {
              ...initialTask,
              prompt: String(request.prompt),
              requested_model: String(request.model_id),
              parameters: request.parameters as object,
              state: paidFailure ? "unconfirmed" : "queued",
            };
            current.tasks.unshift(created as typeof initialTask);
            update();
            if (paidFailure)
              throw {
                code: "result_unconfirmed",
                message: "SYNTHETIC_SECRET_SHOULD_NOT_RENDER",
                retryable: false,
              };
            return created;
          }
          if (command === "reuse_task_parameters") {
            const found = current.tasks.find(
              (item) => item.id === request.task_id,
            );
            if (!found) throw { code: "task_unavailable" };
            return {
              prompt: found.prompt,
              model_id: found.requested_model,
              parameters: structuredClone(found.parameters),
              inputs: [],
            };
          }
          if (command === "retry_local_save") {
            const found = current.tasks.find(
              (item) => item.id === request.task_id,
            );
            if (!found?.local_save_available)
              throw { code: "local_material_missing" };
            found.state = "completed";
            found.local_save_available = false;
            update();
            return found;
          }
          if (command === "delete_history") {
            const ids = request.image_ids as string[];
            current.image_history = current.image_history.filter(
              (image) => !ids.includes(image.id),
            );
            for (const savedTask of current.tasks)
              savedTask.image_ids = savedTask.image_ids.filter(
                (id) => !ids.includes(id),
              );
            update();
            return { deleted: ids.length };
          }
          if (command === "preview_restore") {
            const backup = current.backups.find(
              (item) => item.id === request.backup_group_id,
            )!;
            return {
              id: "synthetic-restore-preview",
              client: backup.client,
              operation: "restore",
              target_version: "synthetic-version",
              paths:
                backup.client === "claude"
                  ? [
                      "/fixture/synthetic-claude-profile.json",
                      "/fixture/synthetic-claude-metadata.json",
                      "/fixture/synthetic-claude-desktop.json",
                    ]
                  : ["/fixture/config", codexCatalog.path],
              claude_configuration_target:
                backup.client === "claude" ? claudeConfigurationTarget : null,
              codex_catalog: null,
              codex_configuration_target:
                backup.client === "codex" ? codexConfigurationTarget : null,
              codex_runtime: null,
              model_id: null,
              context: null,
              restore_owner: {
                account_id: backup.account_id,
                account_label: backup.account_label,
                panel_origin: backup.panel_origin,
                installation_id: backup.installation_id,
              },
              restore_source: {
                backup_group_id: backup.id,
                expires_at: backup.expires_at,
                kind: backup.kind,
              },
              impact: ["Synthetic complete original operation group"],
              redacted_diff: "- mixed fixture state\n+ original fixture group",
              expires_at: new Date(Date.now() + 60000).toISOString(),
              can_apply:
                backup.kind === "switch" || restoreMode === "recoverable",
              can_export_import: false,
              blocking_reason:
                backup.kind === "switch" || restoreMode === "recoverable"
                  ? null
                  : "config_conflict",
            };
          }
          if (command === "restore_client_config") {
            const backup = current.backups.find(
              (item) =>
                item.id ===
                control.lastRequests.preview_restore?.backup_group_id,
            )!;
            if (backup.kind === "switch") {
              current.backups = current.backups.filter(
                (item) => item.id !== backup.id,
              );
              current.clients.find(
                (client) => client.client === backup.client,
              )!.configuration_state = "not_configured";
              update();
            }
            return {
              client: backup.client,
              state: "restored",
              loaded_state: "unconfirmed",
              backup_group_id: null,
            };
          }
          if (command === "apply_client_config") {
            const kind = control.lastRequests.preview_client_config?.client;
            const client = current.clients.find(
              (item) => item.client === kind,
            )!;
            client.configuration_state = "saved";
            client.configuration_revision = "synthetic-saved-configuration";
            client.loaded_state = "restart_required";
            client.connection_state = "not_tested";
            client.loaded_evidence = null;
            client.connection_evidence = null;
            current.backups.push({
              id: "synthetic-config-backup",
              kind: "switch",
              client: client.client,
              account_id: current.session.account!.id,
              account_label: current.session.account!.label,
              panel_origin: current.session.panel_origin!,
              installation_id: "synthetic-installation",
              created_at: "2026-10-22T00:00:00Z",
              expires_at: "2099-01-01T00:00:00Z",
              file_count: client.client === "claude" ? 3 : 2,
              state: "available",
            });
            update();
            return {
              client: client.client,
              state: "saved",
              loaded_state: "restart_required",
              backup_group_id: "synthetic-config-backup",
            };
          }
          if (command === "preview_client_config") {
            const connection = current.connections.find(
              (item) => item.id === request.connection_id,
            )!;
            return {
              id: "synthetic-preview",
              client: request.client,
              operation: "apply",
              target_version: "synthetic-version",
              paths:
                request.client === "codex"
                  ? ["/fixture/config", codexCatalog.path]
                  : [
                      "/fixture/synthetic-claude-profile.json",
                      "/fixture/synthetic-claude-metadata.json",
                      "/fixture/synthetic-claude-desktop.json",
                    ],
              claude_configuration_target:
                request.client === "claude" && !claudeImport
                  ? claudeConfigurationTarget
                  : null,
              codex_catalog: request.client === "codex" ? codexCatalog : null,
              codex_configuration_target:
                request.client === "codex" ? codexConfigurationTarget : null,
              codex_runtime: request.client === "codex" ? codexRuntime : null,
              model_id:
                (request.claude_models as { primary: string } | undefined)
                  ?.primary ?? codexCatalog.default_alias,
              context: {
                account_id: connection.account_id,
                account_label: current.session.account!.label,
                connection_id: connection.id,
                connection_revision: connection.revision,
                credential_revision: connection.credential_revision,
                node_id: connection.node_id,
                node_label: connection.node_label,
                model_revision: "model-revision-1",
              },
              restore_owner: null,
              restore_source: null,
              impact: ["Synthetic fixture configuration only"],
              redacted_diff:
                "- model = old\n+ model = synthetic-text\n+ helper binding = [redacted]",
              expires_at: new Date(Date.now() + 60000).toISOString(),
              can_apply: !claudeImport || request.client !== "claude",
              can_export_import: claudeImport && request.client === "claude",
              blocking_reason:
                claudeImport && request.client === "claude"
                  ? "claude_library_schema_unverified"
                  : null,
            };
          }
          if (command === "export_claude_import") return { saved: true };
          throw { code: "unsupported_fixture_command", retryable: false };
        },
      };
    },
    {
      initial: state,
      models: [imageModel(), syntheticCodexModel()],
      initialTask: task("queued"),
      paidFailure: options.paidFailure ?? false,
      claudeImport: options.claudeImport ?? false,
      restoreMode: options.restoreMode ?? "blocked",
      codexCatalog: syntheticCodexCatalog(),
      codexConfigurationTarget: syntheticCodexConfigurationTarget(),
      codexRuntime: { state: "unverified", cli_version: null, revision: null },
      claudeConfigurationTarget: syntheticClaudeConfigurationTarget(),
    },
  );
}
export async function calls(page: Page, command: string): Promise<number> {
  return page.evaluate(
    (name) =>
      (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
        .counts[name] ?? 0,
    command,
  );
}
export async function pushState(page: Page, state: BootstrapView) {
  await page.evaluate(
    (value) =>
      (
        window as unknown as { __ORBIT_TEST__: FixtureControl }
      ).__ORBIT_TEST__.pushState(value),
    state,
  );
}
