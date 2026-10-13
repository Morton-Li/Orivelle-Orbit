import type { Page } from "@playwright/test";
import type {
  AuthorizationEvent,
  BootstrapView,
  ModelView,
} from "../../src/shared/contracts";
import { bootstrap, imageModel, task } from "../fixtures";

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
  } = {},
) {
  const state = options.state ?? bootstrap();
  const text: ModelView = {
    ...imageModel("synthetic-text"),
    category: "text",
    capabilities: ["openai.responses", "anthropic.messages"],
    image_parameters: null,
    image_billing_mode: null,
    sales_price: null,
  };
  await page.addInitScript(
    ({ initial, models, initialTask, paidFailure, claudeImport }) => {
      // Test-only in-memory IPC. This is injected by Playwright and is never included in the application bundle.
      const testWindow = window as unknown as {
        isTauri: boolean;
        __TAURI_INTERNALS__: {
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
        transformCallback(callback) {
          const id = ++next;
          callbacks.set(id, callback);
          return id;
        },
        async invoke(command, args = {}) {
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
          if (command === "preview_client_config") {
            const connection = current.connections.find(
              (item) => item.id === request.connection_id,
            )!;
            return {
              id: "synthetic-preview",
              client: request.client,
              operation: "apply",
              target_version: "synthetic-version",
              paths: ["/fixture/config"],
              model_id: request.model_id,
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
      models: [imageModel(), text],
      initialTask: task("queued"),
      paidFailure: options.paidFailure ?? false,
      claudeImport: options.claudeImport ?? false,
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
