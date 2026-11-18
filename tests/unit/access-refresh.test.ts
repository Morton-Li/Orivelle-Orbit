import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
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
  orbitSelectStub,
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
    restore_source: null,
    impact: [],
    redacted_diff: "Synthetic Claude configuration only",
    expires_at: null,
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
  bridge.call.mockReset().mockImplementation(async (command) => {
    if (command === "list_models") return nextModels();
    if (command === "preview_client_config") return nextPreview;
    if (command === "bootstrap") return state;
    if (command === "discard_config_preview") return null;
    if (command === "apply_client_config")
      return {
        state: "saved",
        loaded_state: "restart_required",
        snapshot_id: "synthetic-backup",
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
        OrbitSelect: orbitSelectStub,
        ClientRow: true,
        OrbitDialog: {
          props: ["open"],
          template: '<div v-if="open" role="dialog"><slot /></div>',
        },
      },
    },
  });
  for (const role of ["sonnet", "opus", "haiku"]) {
    await wrapper.get(`#claude-${role}`).setValue(claudeModel().id);
  }
  return wrapper.get<HTMLButtonElement>(".access-client .button.primary");
}
function writes() {
  return bridge.call.mock.calls.filter(
    ([command]) => command === "apply_client_config",
  );
}

describe("Codex catalog synchronization status", () => {
  it.each([
    ["updated", null, null],
    ["current", null, null],
    ["blocked", "codex_reasoning_unavailable", "当前推理档位策略不足"],
    ["blocked", "configuration_conflict", null],
    ["blocked", "network_unavailable", null],
    ["blocked", "request_timeout", null],
  ] as const)(
    "notifies about %s without adding inline sync messages",
    async (status, error, message) => {
      const value = bootstrap();
      value.state_revision = state.state_revision + 1;
      value.clients = value.clients.filter(
        (client) => client.client === "codex",
      );
      value.clients[0]!.catalog_sync = {
        state: status,
        checked_at: new Date().toISOString(),
        model_count: 2,
        error,
      };
      value.switch_snapshots = [
        {
          id: "owned-codex-switch",
          client: "codex",
          account_id: value.session.account!.id,
          account_label: "Synthetic account",
          panel_origin: value.connections[0]!.panel_origin,
          installation_id: "synthetic-installation",
          created_at: new Date().toISOString(),
          expires_at: new Date().toISOString(),
          file_count: 2,
          state: "available",
        },
      ];
      state = value;
      expect(useOrbitStore().acceptState(value)).toBe(true);
      await flushPromises();
      const router = createRouter({
        history: createMemoryHistory(),
        routes: [{ path: "/", component: { template: "<main />" } }],
      });
      await router.push("/");
      await router.isReady();
      wrapper = mount(AccessPage, {
        global: {
          plugins: [router],
          stubs: { ClientRow: true, OrbitSelect: orbitSelectStub },
        },
      });
      if (message) {
        expect(useOrbitStore().notice?.text).toContain(message);
        expect(wrapper.text()).not.toContain(message);
        useOrbitStore().clearNotice();
        const repeated = structuredClone(value);
        repeated.state_revision++;
        if (status === "blocked")
          repeated.clients[0]!.catalog_sync!.checked_at = new Date(
            Date.now() + 300000,
          ).toISOString();
        expect(useOrbitStore().acceptState(repeated)).toBe(true);
        expect(useOrbitStore().notice).toBeNull();
      } else {
        expect(useOrbitStore().notice).toBeNull();
      }
      expect(wrapper.text()).not.toContain("每 5 分钟自动同步");
      expect(wrapper.text()).not.toContain("客户端已载入");
      expect(writes()).toHaveLength(0);
    },
  );
  it("does not repeat a saved sync message on startup", () => {
    setActivePinia(createPinia());
    const value = bootstrap();
    value.clients.find((client) => client.client === "codex")!.catalog_sync = {
      state: "updated",
      checked_at: new Date().toISOString(),
      model_count: 8,
      error: null,
    };
    value.switch_snapshots = [
      {
        id: "owned-codex-startup",
        client: "codex",
        account_id: value.session.account!.id,
        account_label: "Synthetic account",
        panel_origin: value.connections[0]!.panel_origin,
        installation_id: "synthetic-installation",
        created_at: new Date().toISOString(),
        expires_at: new Date().toISOString(),
        file_count: 2,
        state: "available",
      },
    ];
    expect(useOrbitStore().acceptState(value)).toBe(true);
    expect(useOrbitStore().notice).toBeNull();
  });
});

describe("Explicit data refresh feedback", () => {
  it.each(["updated", "blocked"] as const)(
    "reports only refresh success when the catalog is %s",
    async (syncState) => {
      const store = useOrbitStore();
      bridge.call.mockImplementation((command) => {
        if (command === "bootstrap") return state;
        if (command === "list_models") return nextModels();
        if (command === "refresh_account") {
          state = structuredClone(state);
          state.state_revision++;
          state.clients[0]!.client = "codex";
          state.clients[0]!.catalog_sync = {
            state: syncState,
            checked_at: new Date().toISOString(),
            model_count: 8,
            error:
              syncState === "blocked" ? "codex_reasoning_unavailable" : null,
          };
          state.switch_snapshots = [
            {
              id: "owned-codex-refresh",
              client: "codex",
              account_id: state.session.account!.id,
              account_label: "Synthetic account",
              panel_origin: state.connections[0]!.panel_origin,
              installation_id: "synthetic-installation",
              created_at: new Date().toISOString(),
              expires_at: new Date().toISOString(),
              file_count: 2,
              state: "available",
            },
          ];
          store.acceptState(state);
          expect(store.notice).toBeNull();
          return state;
        }
        throw new Error("Unexpected synthetic IPC");
      });
      await store.refreshData();
      expect(store.notice).toEqual({ text: "刷新成功。", tone: "success" });
      expect(writes()).toHaveLength(0);
      if (syncState === "blocked") {
        store.clearNotice();
        const background = structuredClone(state);
        background.state_revision++;
        background.clients[0]!.catalog_sync!.checked_at = new Date(
          Date.now() + 300000,
        ).toISOString();
        store.acceptState(background);
        expect(store.notice?.text).toContain("当前推理档位策略不足");
      }
    },
  );

  it.each(["bootstrap", "refresh_account", "list_models"])(
    "reports one refresh failure when %s fails",
    async (failure) => {
      const store = useOrbitStore();
      bridge.call.mockImplementation((command) => {
        if (command === failure)
          throw publicFailure({ code: "network_unavailable" });
        if (command === "bootstrap" || command === "refresh_account")
          return state;
        if (command === "list_models") return nextModels();
        throw new Error("Unexpected synthetic IPC");
      });
      await store.refreshData();
      expect(store.notice).toEqual({
        text: "刷新失败，请稍后重试。",
        tone: "error",
      });
    },
  );

  it("keeps a successful automatic refresh silent", async () => {
    bridge.call.mockImplementation((command) =>
      command === "list_models" ? nextModels() : state,
    );
    const store = useOrbitStore();
    await store.refresh(true);
    expect(store.notice).toBeNull();
  });

  it("refreshes available account data without requiring a selected node", async () => {
    const store = useOrbitStore();
    state = structuredClone(state);
    state.state_revision++;
    state.connections[0]!.node_id = null;
    store.acceptState(state);
    await flushPromises();
    bridge.call.mockReset().mockResolvedValue(state);
    await store.refreshData();
    expect(store.notice?.text).toBe("刷新成功。");
    expect(
      bridge.call.mock.calls.some(([command]) => command === "list_models"),
    ).toBe(false);
  });
});

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
      const alert = wrapper!.get('[role="dialog"] [role="alert"]');
      expect(alert.text()).not.toContain("Private response");
      expect(wrapper!.get('[role="dialog"] .primary').text()).toBe("重新检查");
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
      expect(wrapper!.get('[role="dialog"] [role="status"]').text()).toContain(
        "正在核对 Claude",
      );
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
      expect(
        wrapper!.get('[role="dialog"] [role="alert"]').text(),
      ).not.toContain("预览已失效");
      expect(wrapper!.get('[role="dialog"] .primary').text()).toBe("重新检查");
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
    expect(wrapper!.get('[role="dialog"] [role="alert"]').text()).not.toContain(
      "预览已失效",
    );
    expect(wrapper!.get('[role="dialog"] .primary').text()).toBe("重新检查");
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
    expect(wrapper!.get('[role="dialog"] [role="alert"]').text()).not.toContain(
      "预览已失效",
    );
    expect(wrapper!.get('[role="dialog"] .primary').text()).toBe("重新检查");
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
    expect(wrapper!.get('[role="dialog"] [role="alert"]').text()).not.toContain(
      "预览已失效",
    );
    expect(wrapper!.get('[role="dialog"] .primary').text()).toBe("重新检查");
    expect(writes()).toHaveLength(0);
  });

  it("retains the gated Claude diagnostic export after catalogue synchronization", async () => {
    const value = preview();
    value.can_apply = false;
    value.can_export_import = true;
    value.expires_at = new Date(Date.now() + 60_000).toISOString();
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

describe("access readiness explanations", () => {
  it("explains the missing Claude mappings until all three entries are selected", async () => {
    wrapper = mount(AccessPage, {
      global: {
        stubs: {
          OrbitSelect: orbitSelectStub,
          ClientRow: true,
          ConfigReviewDialog: true,
        },
      },
    });
    expect(wrapper.text()).toContain(
      "请为 Sonnet、Opus、Haiku 分别选择实际调用的模型",
    );
    expect(
      wrapper.get<HTMLButtonElement>(".access-client .button.primary").element
        .disabled,
    ).toBe(true);
    for (const role of ["sonnet", "opus", "haiku"])
      await wrapper.get(`#claude-${role}`).setValue(claudeModel().id);
    expect(
      wrapper.get<HTMLButtonElement>(".access-client .button.primary").element
        .disabled,
    ).toBe(false);
    expect(wrapper.find("#connect-reason-claude").exists()).toBe(false);
  });
  it("prefills only unambiguous model families and leaves ambiguous choices to the user", async () => {
    const store = useOrbitStore();
    store.models = ["sonnet-a", "sonnet-b", "opus-a", "haiku-a"].map((id) => ({
      ...claudeModel(),
      id,
      label: id,
    }));
    wrapper = mount(AccessPage, {
      global: {
        stubs: {
          OrbitSelect: orbitSelectStub,
          ClientRow: true,
          ConfigReviewDialog: true,
        },
      },
    });
    expect(wrapper.get<HTMLSelectElement>("#claude-sonnet").element.value).toBe(
      "",
    );
    expect(wrapper.get<HTMLSelectElement>("#claude-opus").element.value).toBe(
      "opus-a",
    );
    expect(wrapper.get<HTMLSelectElement>("#claude-haiku").element.value).toBe(
      "haiku-a",
    );
    expect(
      bridge.call.mock.calls.filter(
        ([command]) =>
          command === "apply_client_config" ||
          command === "preview_client_config",
      ),
    ).toHaveLength(0);
  });
});
