import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import ConfigReviewDialog from "../../src/components/ConfigReviewDialog.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import { publicFailure } from "../../src/shared/bridge";
import { configPolicyMessage } from "../../src/shared/format";
import {
  CAPABILITIES,
  type SwitchSnapshot,
  type ConfigPreview,
} from "../../src/shared/contracts";
import {
  bootstrap,
  imageModel,
  syntheticClaudeConfigurationTarget,
} from "../fixtures";

let view: VueWrapper | undefined;
function state(account = "synthetic-account-a", revision = 1) {
  const value = bootstrap(account, revision);
  value.clients = value.clients.filter((client) => client.client === "claude");
  return value;
}
function backup(): SwitchSnapshot {
  return {
    id: "selected-backup",
    client: "claude",
    account_id: "synthetic-account-a",
    account_label: "Synthetic original account",
    panel_origin: "https://panel.invalid",
    installation_id: "synthetic-installation",
    created_at: new Date(Date.now() - 60_000).toISOString(),
    expires_at: new Date(Date.now() + 600_000).toISOString(),
    file_count: 3,
    state: "available",
  };
}
function preview(
  operation: "apply" | "restore",
  source: SwitchSnapshot,
): ConfigPreview {
  const base = {
    id: "claude-confirmation",
    client: "claude" as const,
    target_version: "synthetic-version",
    paths: [
      "/fixture/main.json",
      "/fixture/_meta.json",
      "/fixture/profile.json",
    ],
    impact: [],
    redacted_diff: "Synthetic redacted configuration",
    expires_at: new Date(Date.now() + 60_000).toISOString(),
    can_apply: true,
    can_export_import: false,
    blocking_reason: null,
    claude_configuration_target: syntheticClaudeConfigurationTarget(),
    codex_configuration_target: null,
    codex_catalog: null,
    codex_runtime: null,
  };
  if (operation === "restore") {
    return {
      ...base,
      operation,
      context: null,
      model_id: null,
      restore_owner: {
        account_id: source.account_id,
        account_label: source.account_label,
        panel_origin: source.panel_origin,
        installation_id: source.installation_id,
      },
      restore_source: {
        snapshot_id: source.id,
        expires_at: source.expires_at,
      },
    };
  }
  return {
    ...base,
    operation,
    model_id: "synthetic-claude-model",
    context: {
      account_id: "synthetic-account-a",
      account_label: "Synthetic account",
      connection_id: "connection-synthetic-account-a",
      connection_revision: 1,
      credential_revision: "credential-revision-1",
      node_id: "node-a",
      node_label: "Synthetic node",
      model_revision: "model-revision-1",
    },
    restore_owner: null,
    restore_source: null,
  };
}
function show(operation: "apply" | "restore" = "apply") {
  const source = backup();
  useOrbitStore().state.switch_snapshots = [source];
  view = mount(ConfigReviewDialog, {
    props: { preview: preview(operation, source) },
    global: { stubs: { OrbitDialog: { template: "<div><slot /></div>" } } },
  });
  const button = view.get<HTMLButtonElement>(".button.primary");
  expect(button.element.disabled).toBe(false);
  return button;
}
const signatureFailure = () =>
  publicFailure({
    code: "claude_signature_verification_timeout",
    message: "RAW_PRIVATE_DIAGNOSTIC_MUST_NOT_APPEAR",
  });
beforeEach(async () => {
  setActivePinia(createPinia());
  bridge.call.mockReset().mockResolvedValue([]);
  const store = useOrbitStore();
  store.acceptState(state());
  await flushPromises();
  store.models = [
    {
      ...imageModel("synthetic-claude-model"),
      category: "text",
      capabilities: [CAPABILITIES.messages],
      execution_profile: null,
      image_billing_mode: null,
      image_parameters: null,
    },
  ];
  bridge.call.mockClear();
});
afterEach(() => {
  view?.unmount();
  view = undefined;
});
describe("Claude failed confirmation refreshes recovery state", () => {
  it.each(["apply", "restore"] as const)(
    "shows pending recovery after failed %s without replaying the write",
    async (operation) => {
      const refreshed = state("synthetic-account-a", 2);
      const pending = { ...backup(), state: "conflict" as const };
      refreshed.switch_snapshots = [pending];
      refreshed.clients[0]!.configuration_state = "conflict";
      bridge.call
        .mockRejectedValueOnce(signatureFailure())
        .mockResolvedValueOnce(refreshed);
      const button = show(operation);
      await button.trigger("click");
      await flushPromises();
      expect(bridge.call.mock.calls).toEqual([
        [
          operation === "apply"
            ? "apply_client_config"
            : "restore_client_config",
          { preview_id: "claude-confirmation", confirmed: true },
        ],
        ["bootstrap", {}],
      ]);
      const store = useOrbitStore();
      expect(store.switchSnapshots).toEqual([pending]);
      expect(store.state.clients[0]!.configuration_state).toBe("conflict");
      expect(store.notice?.text).toBe(
        configPolicyMessage("claude_signature_verification_timeout"),
      );
      expect(button.element.disabled).toBe(true);
      await button.trigger("click");
      expect(bridge.call).toHaveBeenCalledTimes(2);
      expect(view!.emitted("applied")).toBeUndefined();
    },
  );

  it("keeps the newer refresh error and retires the failed confirmation when local refresh fails", async () => {
    bridge.call.mockRejectedValueOnce(signatureFailure()).mockRejectedValueOnce(
      publicFailure({
        code: "local_state_unavailable",
        message: "RAW_PRIVATE_REFRESH_ERROR",
      }),
    );
    const button = show();
    await button.trigger("click");
    await flushPromises();
    expect(bridge.call.mock.calls.map(([command]) => command)).toEqual([
      "apply_client_config",
      "bootstrap",
    ]);
    expect(useOrbitStore().notice?.text).toContain("local_state_unavailable");
    expect(useOrbitStore().notice?.text).not.toContain("RAW_PRIVATE");
    expect(button.element.disabled).toBe(true);
    await button.trigger("click");
    expect(bridge.call).toHaveBeenCalledTimes(2);
  });

  it("does not put the old error over a new notice while refresh changes accounts", async () => {
    let resolve!: (value: ReturnType<typeof state>) => void;
    bridge.call.mockImplementation((command: string) => {
      if (command === "apply_client_config")
        return Promise.reject(signatureFailure());
      if (command === "bootstrap")
        return new Promise((done) => {
          resolve = done;
        });
      if (command === "list_models") return Promise.resolve([]);
      throw new Error("Unexpected synthetic command");
    });
    const button = show();
    await button.trigger("click");
    await flushPromises();
    expect(bridge.call.mock.calls.map(([command]) => command)).toEqual([
      "apply_client_config",
      "bootstrap",
    ]);
    const store = useOrbitStore();
    store.notify("新账户通知");
    resolve(state("synthetic-account-b", 2));
    await flushPromises();
    expect(store.notice?.text).toBe("新账户通知");
    expect(store.account?.id).toBe("synthetic-account-b");
    expect(bridge.call.mock.calls.map(([command]) => command)).toEqual([
      "apply_client_config",
      "bootstrap",
      "list_models",
    ]);
    expect(button.element.disabled).toBe(true);
  });
});
