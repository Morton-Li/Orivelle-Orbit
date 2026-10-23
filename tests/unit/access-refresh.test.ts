import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
}));
vi.mock("../../src/shared/bridge", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import AccessPage from "../../src/pages/AccessPage.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import { publicFailure } from "../../src/shared/bridge";
import {
  CAPABILITIES,
  type BootstrapView,
  type ConfigPreview,
  type ModelView,
} from "../../src/shared/contracts";
import {
  bootstrap,
  imageModel,
  syntheticClaudeConfigurationTarget,
} from "../fixtures";

function claudeModel(revision = "R1"): ModelView {
  return {
    ...imageModel("synthetic-claude"),
    category: "text",
    capabilities: [CAPABILITIES.messages],
    revision,
    image_parameters: null,
    image_billing_mode: null,
  };
}
function claudeState(
  account = "synthetic-account-a",
  revision = 1,
): BootstrapView {
  const state = bootstrap(account, revision);
  state.clients = state.clients.filter((client) => client.client === "claude");
  return state;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function preview(): ConfigPreview {
  const connection = useOrbitStore().activeConnection!;
  return {
    id: "synthetic-claude-preview",
    operation: "apply",
    client: "claude",
    target_version: "synthetic-version",
    paths: [
      "/fixture/profile.json",
      "/fixture/_meta.json",
      "/fixture/claude_desktop_config.json",
    ],
    claude_configuration_target: syntheticClaudeConfigurationTarget(),
    codex_configuration_target: null,
    codex_catalog: null,
    codex_runtime: null,
    model_id: claudeModel().id,
    context: {
      account_id: connection.account_id,
      account_label: "Synthetic account",
      connection_id: connection.id,
      connection_revision: connection.revision,
      credential_revision: connection.credential_revision!,
      node_id: connection.node_id!,
      node_label: "Synthetic node",
      model_revision: "R2",
    },
    restore_owner: null,
    impact: [],
    redacted_diff: "Synthetic Claude configuration only",
    expires_at: new Date(Date.now() + 60_000).toISOString(),
    can_apply: true,
    can_export_import: false,
    blocking_reason: null,
  };
}
let wrapper: VueWrapper | undefined;
let state: BootstrapView;
let nextModels: () => ModelView[] | Promise<ModelView[]>;
let nextPreview: ConfigPreview | Promise<ConfigPreview>;
beforeEach(async () => {
  setActivePinia(createPinia());
  state = claudeState();
  nextModels = () => [claudeModel()];
  bridge.call.mockReset().mockImplementation((command) => {
    if (command === "list_models") return nextModels();
    if (command === "preview_client_config") return nextPreview;
    if (command === "bootstrap") return state;
    if (command === "apply_client_config")
      return {
        state: "saved",
        loaded_state: "restart_required",
        backup_group_id: "synthetic-backup",
      };
    throw new Error("Unexpected synthetic IPC");
  });
  useOrbitStore().acceptState(state);
  await flushPromises();
  nextModels = () => [claudeModel("R2")];
  nextPreview = preview();
  bridge.call.mockClear();
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});
async function showAccess() {
  wrapper = mount(AccessPage, {
    global: {
      stubs: {
        ClientRow: true,
        OrbitDialog: {
          props: ["open"],
          template: '<div v-if="open" role="dialog"><slot /></div>',
        },
      },
    },
  });
  await wrapper.get("#model-claude").setValue(claudeModel().id);
  return wrapper.get<HTMLButtonElement>(".access-client .button.primary");
}
function writes() {
  return bridge.call.mock.calls.filter(
    ([command]) => command === "apply_client_config",
  );
}

describe("Claude preview after a Panel model refresh", () => {
  it("synchronizes R2 before first displaying its confirmation and keeps saved distinct from loaded", async () => {
    const button = await showAccess();
    await button.trigger("click");
    await flushPromises();
    const confirm = wrapper!.get<HTMLButtonElement>(
      '[role="dialog"] .button.primary',
    );
    expect(useOrbitStore().models[0]?.revision).toBe("R2");
    expect(confirm.element.disabled).toBe(false);
    expect(writes()).toHaveLength(0);
    await confirm.trigger("click");
    await flushPromises();
    expect(writes()).toEqual([
      [
        "apply_client_config",
        { preview_id: "synthetic-claude-preview", confirmed: true },
      ],
    ]);
    expect(useOrbitStore().notice?.text).toContain("配置已保存");
    expect(useOrbitStore().notice?.text).not.toContain("连接验证通过");
  });

  it.each(["failure", "changed", "removed"] as const)(
    "discards the preview when the model sync is %s",
    async (outcome) => {
      nextModels = () => {
        if (outcome === "failure")
          throw publicFailure({
            code: "network_unavailable",
            message: "Private response",
          });
        return outcome === "removed" ? [] : [claudeModel("R3")];
      };
      const button = await showAccess();
      await button.trigger("click");
      await flushPromises();
      expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
      expect(useOrbitStore().notice).not.toBeNull();
      expect(useOrbitStore().notice?.text).not.toContain("Private response");
      if (outcome === "failure") {
        expect(useOrbitStore().notice?.tone).toBe("error");
        expect(useOrbitStore().notice?.text).toContain("network_unavailable");
      }
      expect(writes()).toHaveLength(0);
    },
  );

  it.each(["account", "node", "credential"] as const)(
    "drops an in-flight preview after the %s changes and fences double clicks during model sync",
    async (change) => {
      const pending = deferred<ModelView[]>();
      nextModels = vi
        .fn()
        .mockReturnValueOnce(pending.promise)
        .mockResolvedValue([claudeModel("R3")]);
      const button = await showAccess();
      await button.trigger("click");
      await flushPromises();
      expect(button.element.disabled).toBe(true);
      expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
      await button.trigger("click");
      expect(
        bridge.call.mock.calls.filter(
          ([command]) => command === "preview_client_config",
        ),
      ).toHaveLength(1);
      state = claudeState(
        change === "account" ? "synthetic-account-b" : "synthetic-account-a",
        2,
      );
      if (change !== "account") {
        state.connections[0]!.revision++;
        if (change === "node") state.connections[0]!.node_id = "node-b";
        else state.connections[0]!.credential_revision = "credential-R2";
      }
      useOrbitStore().acceptState(state);
      pending.resolve([claudeModel("R2")]);
      await flushPromises();
      expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
      expect(writes()).toHaveLength(0);
    },
  );

  it("drops a preview whose model read was superseded by a newer same-scope read", async () => {
    const pending = deferred<ModelView[]>();
    nextModels = vi
      .fn()
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValue([claudeModel("R2")]);
    const button = await showAccess();
    await button.trigger("click");
    await flushPromises();
    await useOrbitStore().loadModels();
    pending.resolve([claudeModel("R2")]);
    await flushPromises();
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
    expect(writes()).toHaveLength(0);
  });

  it("discards an old account's native preview before reading the new account's models", async () => {
    const pending = deferred<ConfigPreview>();
    const old = preview();
    nextPreview = pending.promise;
    const button = await showAccess();
    await button.trigger("click");
    state = claudeState("synthetic-account-b", 2);
    useOrbitStore().acceptState(state);
    await flushPromises();
    const reads = bridge.call.mock.calls.filter(
      ([command]) => command === "list_models",
    ).length;
    pending.resolve(old);
    await flushPromises();
    expect(
      bridge.call.mock.calls.filter(([command]) => command === "list_models"),
    ).toHaveLength(reads);
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
    expect(writes()).toHaveLength(0);
  });

  it("permanently drops the old preview after leaving and returning to the same account scope", async () => {
    const pending = deferred<ModelView[]>();
    nextModels = vi
      .fn()
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValue([claudeModel("R2")]);
    const button = await showAccess();
    await button.trigger("click");
    await flushPromises();
    const oldScope = useOrbitStore().scope;
    useOrbitStore().acceptState(claudeState("synthetic-account-b", 2));
    state = claudeState("synthetic-account-a", 3);
    useOrbitStore().acceptState(state);
    expect(useOrbitStore().scope).toBe(oldScope);
    pending.resolve([claudeModel("R2")]);
    await flushPromises();
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false);
    expect(writes()).toHaveLength(0);
  });

  it("retains the gated Claude diagnostic export after catalogue synchronization", async () => {
    const value = preview();
    value.can_apply = false;
    value.can_export_import = true;
    value.claude_configuration_target = null;
    value.blocking_reason = "schema_unverified";
    nextPreview = value;
    useOrbitStore().state.clients[0]!.configuration_access = "unverified";
    useOrbitStore().state.clients[0]!.configuration_access_revision = null;
    const button = await showAccess();
    await button.trigger("click");
    await flushPromises();
    const dialog = wrapper!.get('[role="dialog"]');
    expect(
      dialog.get<HTMLButtonElement>(".button.primary").element.disabled,
    ).toBe(true);
    const exportButton = dialog
      .findAll<HTMLButtonElement>("button")
      .find((button) => button.text().includes("导出 Claude"))!;
    expect(exportButton.element.disabled).toBe(false);
    expect(writes()).toHaveLength(0);
  });
});
