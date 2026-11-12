import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";

const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
}));
vi.mock("../../src/shared/bridge", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));

import ConfigReviewDialog from "../../src/components/ConfigReviewDialog.vue";
import ClientRow from "../../src/components/ClientRow.vue";
import AccessPage from "../../src/pages/AccessPage.vue";
import { OrbitError } from "../../src/shared/bridge";
import type { ClientView, ConfigPreview } from "../../src/shared/contracts";
import { useOrbitStore } from "../../src/stores/orbit";
import {
  bootstrap,
  orbitSelectStub,
  syntheticClaudeConfigurationTarget,
  syntheticCodexCatalog,
  syntheticCodexConfigurationTarget,
  syntheticCodexModel,
  syntheticCodexRuntime,
} from "../fixtures";

let wrapper: VueWrapper | undefined;
const model = syntheticCodexModel();
beforeEach(async () => {
  setActivePinia(createPinia());
  bridge.call.mockReset().mockResolvedValue([model]);
  useOrbitStore().acceptState(bootstrap());
  await flushPromises();
  bridge.call.mockClear();
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});
function preview(client: "codex" | "claude" = "codex"): ConfigPreview {
  const connection = useOrbitStore().activeConnection!;
  return {
    id: "11111111-1111-4111-8111-111111111111",
    operation: "apply",
    client,
    target_version: "synthetic-version",
    paths:
      client === "codex"
        ? ["/fixture/config", "/fixture/codex-model-catalog.json"]
        : [
            "/fixture/synthetic-claude-profile.json",
            "/fixture/synthetic-claude-metadata.json",
            "/fixture/synthetic-claude-desktop.json",
          ],
    claude_configuration_target:
      client === "claude" ? syntheticClaudeConfigurationTarget() : null,
    codex_catalog: client === "codex" ? syntheticCodexCatalog() : null,
    codex_configuration_target:
      client === "codex" ? syntheticCodexConfigurationTarget() : null,
    codex_runtime:
      client === "codex"
        ? { state: "unverified", cli_version: null, revision: null }
        : null,
    model_id: model.id,
    context: {
      account_id: connection.account_id,
      account_label: "Synthetic account",
      connection_id: connection.id,
      connection_revision: connection.revision,
      credential_revision: connection.credential_revision!,
      node_id: connection.node_id!,
      node_label: "Synthetic node",
      model_revision: model.revision,
    },
    restore_owner: null,
    restore_source: null,
    impact: [],
    redacted_diff: "synthetic config",
    expires_at: new Date(Date.now() + 60_000).toISOString(),
    can_apply: true,
    can_export_import: false,
    blocking_reason: null,
  };
}
function show(value: ConfigPreview) {
  wrapper = mount(ConfigReviewDialog, {
    props: { preview: value },
    global: { stubs: { OrbitDialog: { template: "<div><slot /></div>" } } },
  });
  return wrapper;
}

describe("configuration confirmation follows current policy", () => {
  it.each(
    (["codex", "claude"] as const).flatMap((kind) =>
      (["managed", "blocked", "unverified", "changed target"] as const).map(
        (qualification) => ({ kind, qualification }),
      ),
    ),
  )(
    "retires a $kind confirmation after local qualification becomes $qualification, including after it clears again",
    async ({ kind, qualification }) => {
      useOrbitStore().state.clients.find(
        (client) => client.client === kind,
      )!.managed = "unknown";
      const view = show(preview(kind));
      const apply = view.get<HTMLButtonElement>(".button.primary");
      expect(apply.element.disabled).toBe(false);
      if (kind === "claude") {
        expect(view.findAll(".path-list li")).toHaveLength(3);
        expect(view.text()).toContain("Chat、Code 与 Cowork");
        expect(view.text()).toContain("远端组织策略尚未确认");
        expect(view.text()).not.toContain("客户端载入已确认");
      }
      const changed = bootstrap("synthetic-account-a", 2);
      const client = changed.clients.find((item) => item.client === kind)!;
      if (qualification === "managed") client.managed = "yes";
      else if (qualification === "changed target")
        client.configuration_access_revision = "synthetic-new-qualification";
      else client.configuration_access = qualification;
      useOrbitStore().acceptState(changed);
      await nextTick();
      expect(apply.element.disabled).toBe(true);
      await apply.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();
      useOrbitStore().acceptState(bootstrap("synthetic-account-a", 3));
      await nextTick();
      expect(apply.element.disabled).toBe(true);
      await apply.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();

      const fresh = {
        ...preview(kind),
        id: "22222222-2222-4222-8222-222222222222",
      };
      await view.setProps({ preview: fresh });
      expect(apply.element.disabled).toBe(false);
      bridge.call.mockImplementation(async (command) =>
        command === "bootstrap"
          ? bootstrap("synthetic-account-a", 4)
          : {
              client: kind,
              state: "saved",
              loaded_state: "restart_required",
              snapshot_id: "synthetic-backup",
            },
      );
      await apply.trigger("click");
      await flushPromises();
      expect(bridge.call).toHaveBeenCalledWith("apply_client_config", {
        preview_id: fresh.id,
        confirmed: true,
      });
      expect(
        bridge.call.mock.calls.filter(
          ([command]) => command === "apply_client_config",
        ),
      ).toHaveLength(1);

      await view.setProps({ preview: preview(kind) });
      expect(apply.element.disabled).toBe(true);
      await apply.trigger("click");
      expect(
        bridge.call.mock.calls.filter(
          ([command]) => command === "apply_client_config",
        ),
      ).toHaveLength(1);
    },
  );

  it("also blocks the existing Claude diagnostic export when policy is unconfirmed", async () => {
    const value = preview("claude");
    value.claude_configuration_target = null;
    value.can_apply = false;
    value.can_export_import = true;
    value.blocking_reason = "schema_unverified";
    const client = useOrbitStore().state.clients[1]!;
    client.configuration_access = "unverified";
    client.configuration_access_revision = null;
    const view = show(value);
    const exportButton = view
      .findAll<HTMLButtonElement>("button")
      .find((button) => button.text().includes("导出 Claude"))!;
    expect(exportButton.element.disabled).toBe(false);
    expect(
      view.get<HTMLButtonElement>(".button.primary").element.disabled,
    ).toBe(true);
    expect(view.text()).toContain("不代表可直接导入");
    expect(view.text()).not.toContain("官方导入文件");
    const changed = bootstrap("synthetic-account-a", 2);
    changed.clients[1]!.managed = "unknown";
    useOrbitStore().acceptState(changed);
    await nextTick();
    expect(exportButton.element.disabled).toBe(true);
    await exportButton.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it("retires a rejected confirmation without retrying or exposing the native policy message", async () => {
    const view = show(preview());
    bridge.call.mockRejectedValueOnce(
      new OrbitError(
        "managed_policy_changed",
        "synthetic-private-policy-value",
      ),
    );
    const apply = view.get<HTMLButtonElement>(".button.primary");
    await apply.trigger("click");
    await flushPromises();
    expect(bridge.call).toHaveBeenCalledTimes(1);
    expect(apply.element.disabled).toBe(true);
    expect(useOrbitStore().notice?.text).toContain("组织管理策略已变化");
    expect(useOrbitStore().notice?.text).not.toContain(
      "synthetic-private-policy-value",
    );
    await apply.trigger("click");
    expect(bridge.call).toHaveBeenCalledTimes(1);
  });
});

describe("Claude configuration target confirmation", () => {
  it.each([
    "missing access",
    "missing access revision",
    "missing target",
    "empty target revision",
    "different target revision",
    "different target app",
  ] as const)(
    "rejects %s without accepting old payloads as a write qualification",
    async (missing) => {
      const value = preview("claude");
      const client = useOrbitStore().state.clients[1]!;
      client.managed = "unknown";
      if (missing === "missing access")
        delete (client as Partial<ClientView>).configuration_access;
      else if (missing === "missing access revision")
        delete (client as Partial<ClientView>).configuration_access_revision;
      else if (missing === "missing target")
        delete (value as Partial<ConfigPreview>).claude_configuration_target;
      else if (missing === "empty target revision")
        value.claude_configuration_target!.revision = "";
      else if (missing === "different target revision")
        value.claude_configuration_target!.revision = "synthetic-other-target";
      else
        value.claude_configuration_target!.app_version =
          "synthetic-other-version";
      const view = show(value);
      const button = view.get<HTMLButtonElement>(".button.primary");
      expect(button.element.disabled).toBe(true);
      await button.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();
      await view.setProps({
        preview: {
          ...value,
          claude_configuration_target: syntheticClaudeConfigurationTarget(),
        },
      });
      expect(button.element.disabled).toBe(true);
    },
  );

  it("permanently retires a confirmation when the installed Claude version changes, even if it returns", async () => {
    const view = show(preview("claude"));
    const button = view.get<HTMLButtonElement>(".button.primary");
    expect(button.element.disabled).toBe(false);
    const client = useOrbitStore().state.clients[1]!;
    client.version = "synthetic-other-version";
    await nextTick();
    expect(button.element.disabled).toBe(true);
    client.version = "synthetic-version";
    await nextTick();
    expect(button.element.disabled).toBe(true);
    await button.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
  });
});

describe("Codex catalog confirmation", () => {
  it.each([
    "missing access",
    "missing access revision",
    "missing preview target",
    "empty target revision",
    "different target CLI",
  ] as const)(
    "rejects %s even when old or synthetic runtime metadata says verified",
    async (missing) => {
      const value = preview();
      value.codex_runtime = syntheticCodexRuntime();
      const client = useOrbitStore().state.clients[0]!;
      if (missing === "missing access")
        delete (client as Partial<ClientView>).configuration_access;
      else if (missing === "missing access revision")
        delete (client as Partial<ClientView>).configuration_access_revision;
      else if (missing === "missing preview target")
        delete (value as Partial<ConfigPreview>).codex_configuration_target;
      else if (missing === "empty target revision")
        value.codex_configuration_target!.revision = "";
      else
        value.codex_configuration_target!.cli_version = "synthetic-other-CLI";
      const view = show(value);
      const apply = view.get<HTMLButtonElement>(".button.primary");
      expect(apply.element.disabled).toBe(true);
      await apply.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();
      if (missing === "different target CLI") {
        await view.setProps({
          preview: {
            ...value,
            codex_configuration_target: syntheticCodexConfigurationTarget(),
          },
        });
        expect(apply.element.disabled).toBe(true);
        await apply.trigger("click");
        expect(bridge.call).not.toHaveBeenCalled();
      }
    },
  );

  it("shows saved, restart and connection states without treating local qualification as loaded evidence", () => {
    const store = useOrbitStore();
    store.state.clients[0]!.managed = "unknown";
    wrapper = mount(ClientRow, {
      props: {
        client: store.state.clients[0]!,
        connection: store.activeConnection,
        detected: true,
      },
      global: { stubs: { RouterLink: { template: "<a><slot /></a>" } } },
    });
    expect(wrapper.text()).not.toContain("可预览本地用户配置");
    expect(wrapper.text()).toContain("远端组织策略未确认");
    expect(wrapper.text()).toContain("配置已保存");
    expect(wrapper.text()).toContain("待重启 / 载入未确认");
    expect(wrapper.text()).toContain("连接未验证");
    expect(wrapper.text()).not.toContain("客户端载入已确认");
    expect(wrapper.text()).not.toContain("连接验证通过");
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it.each(["codex", "claude"] as const)(
    "enables the %s preview entry for current local write eligibility with unknown remote policy",
    async (kind) => {
      const store = useOrbitStore();
      const client = store.state.clients.find((item) => item.client === kind)!;
      client.managed = "unknown";
      wrapper = mount(AccessPage, {
        global: {
          stubs: {
            OrbitSelect: orbitSelectStub,
            ConfigReviewDialog: true,
            OrbitDialog: true,
            ClientRow: true,
          },
        },
      });
      if (kind === "claude") {
        for (const role of ["sonnet", "opus", "haiku"]) {
          await wrapper.get(`#claude-${role}`).setValue(model.id);
        }
      }
      const button = wrapper
        .findAll(".access-client")
        [kind === "codex" ? 0 : 1]!.get<HTMLButtonElement>(".button.primary");
      expect(button.element.disabled).toBe(false);
      for (const access of ["blocked", "unverified"] as const) {
        client.configuration_access = access;
        await nextTick();
        expect(button.element.disabled).toBe(true);
        await button.trigger("click");
      }
      client.configuration_access = "allowed";
      client.managed = "yes";
      await nextTick();
      expect(button.element.disabled).toBe(true);
      await button.trigger("click");
      client.managed = "unknown";
      delete (client as Partial<ClientView>).configuration_access;
      await nextTick();
      expect(button.element.disabled).toBe(true);
      await button.trigger("click");
      expect(
        bridge.call.mock.calls.filter(
          ([command]) =>
            !["probe_nodes", "cancel_node_probes"].includes(command),
        ),
      ).toEqual([]);
      client.configuration_access = "allowed";
      await nextTick();
      expect(button.element.disabled).toBe(false);
      bridge.call.mockResolvedValueOnce(preview(kind));
      await button.trigger("click");
      await flushPromises();
      expect(bridge.call).toHaveBeenCalledWith("preview_client_config", {
        connection_id: store.activeConnection!.id,
        expected_revision: store.activeConnection!.revision,
        client: kind,
        ...(kind === "claude"
          ? {
              claude_mapping: {
                sonnet: model.id,
                opus: model.id,
                haiku: model.id,
              },
            }
          : {}),
      });
    },
  );

  it.each(
    (["codex", "claude"] as const).flatMap((kind) =>
      (["restored", "conflict"] as const).map((state) => ({ kind, state })),
    ),
  )(
    "confirms an eligible $kind restore while logged out and reports $state using its original backup owner",
    async ({ kind, state }) => {
      const store = useOrbitStore();
      const original = preview(kind);
      const owner = {
        account_id: "synthetic-account-a",
        account_label: "Synthetic original account",
        panel_origin: "https://panel.invalid",
        installation_id: "synthetic-installation",
      };
      store.state.clients.find((item) => item.client === kind)!.managed =
        "unknown";
      store.state.session.status = "logged_out";
      store.state.session.account = null;
      store.state.session.inference_credential_status = "unavailable";
      store.state.switch_snapshots.push({
        ...owner,
        id: "synthetic-backup",
        client: kind,
        created_at: "2026-10-22T00:00:00Z",
        expires_at: "2099-01-01T00:00:00Z",
        file_count: kind === "claude" ? 3 : 2,
        state: "available",
      });
      const value: ConfigPreview = {
        ...original,
        operation: "restore",
        model_id: null,
        context: null,
        restore_owner: owner,
        restore_source: {
          snapshot_id: "synthetic-backup",
          expires_at: "2099-01-01T00:00:00Z",
        },
        codex_catalog: null,
        codex_runtime: null,
      };
      const view = show(value);
      const button = view.get<HTMLButtonElement>(".button.primary");
      expect(store.ready).toBe(false);
      expect(button.element.disabled).toBe(false);
      expect(button.text()).toBe("确认恢复配置");
      expect(view.text()).toContain("远端组织策略尚未确认");
      expect(view.text()).not.toContain("本次预览的执行来源已核验");
      bridge.call.mockImplementation(async (command) =>
        command === "bootstrap"
          ? bootstrap("synthetic-account-a", 2)
          : {
              client: kind,
              state,
              loaded_state: "unconfirmed",
              snapshot_id: null,
            },
      );
      await button.trigger("click");
      await flushPromises();
      expect(bridge.call).toHaveBeenCalledWith("restore_client_config", {
        preview_id: value.id,
        confirmed: true,
      });
      expect(store.notice).toEqual({
        text:
          state === "conflict"
            ? "配置冲突"
            : "原配置已恢复。请完整退出目标应用并重新打开，载入和连接仍待确认。",
        tone: state === "conflict" ? "error" : "neutral",
      });
    },
  );

  it("shows the compiled menu and explicit defaults separately from client loading and connection", () => {
    const value = preview();
    value.codex_runtime = syntheticCodexRuntime();
    value.codex_catalog!.aliases.push("synthetic-second");
    value.codex_catalog!.default_alias = "synthetic-second";
    useOrbitStore().models.push(syntheticCodexModel("synthetic-second"));
    const view = show(value);
    expect(
      view
        .get('ul[aria-label="模型菜单调用名称（本次目录）"]')
        .findAll("li")
        .map((item) => item.text()),
    ).toEqual(["synthetic-text", "synthetic-second"]);
    const context = view.findAll("dl")[2]!;
    expect(context.text()).toContain("默认模型调用名称synthetic-second");
    expect(context.text()).toContain("默认推理档位high");
    expect(context.text()).toContain("0.163.0-alpha.10.2");
    expect(view.text()).toContain("本次预览的执行来源已核验");
    expect(view.text()).toContain("不代表客户端已载入或连接通过");
    expect(view.text()).toContain("/fixture/config");
    expect(view.text()).toContain("/fixture/codex-model-catalog.json");
    expect(view.text()).not.toContain(value.codex_catalog!.source_sha256);
    expect(view.text()).not.toContain(value.codex_catalog!.sha256);
    expect(view.text()).not.toContain(
      value.codex_configuration_target!.revision,
    );
    expect(
      view.get<HTMLButtonElement>(".button.primary").element.disabled,
    ).toBe(false);
    expect(bridge.call).not.toHaveBeenCalled();
  });

  const independentRuntimes: {
    name: string;
    runtime: ConfigPreview["codex_runtime"];
    checked: boolean;
  }[] = [
    { name: "absent", runtime: null, checked: false },
    {
      name: "unverified",
      runtime: { ...syntheticCodexRuntime(), state: "unverified" },
      checked: false,
    },
    {
      name: "a different CLI version",
      runtime: {
        ...syntheticCodexRuntime(),
        cli_version: "synthetic-other-version",
      },
      checked: true,
    },
    {
      name: "missing its revision",
      runtime: { ...syntheticCodexRuntime(), revision: null },
      checked: false,
    },
    {
      name: "missing its CLI version",
      runtime: { ...syntheticCodexRuntime(), cli_version: null },
      checked: false,
    },
  ];
  it.each(independentRuntimes)(
    "can save qualified local configuration while independently displaying runtime metadata that is $name",
    async ({ runtime, checked }) => {
      const value = preview();
      value.codex_runtime = runtime;
      useOrbitStore().state.clients[0]!.managed = "unknown";
      const view = show(value);
      const apply = view.get<HTMLButtonElement>(".button.primary");
      expect(value.can_apply).toBe(true);
      expect(apply.element.disabled).toBe(false);
      expect(apply.text()).toBe("确认保存本地配置");
      expect(view.text()).toContain(
        "远端组织策略尚未确认，可能影响本地配置生效",
      );
      expect(view.text()).toContain("默认用户配置文件");
      expect(view.text()).toContain("不代表客户端已载入或连接通过");
      expect(view.text().includes("本次预览的执行来源已核验")).toBe(checked);
      bridge.call.mockRejectedValueOnce(
        new OrbitError(
          "codex_configuration_target_changed",
          "synthetic-private-target",
        ),
      );
      await apply.trigger("click");
      await flushPromises();
      expect(bridge.call).toHaveBeenCalledWith("apply_client_config", {
        preview_id: value.id,
        confirmed: true,
      });
      expect(apply.element.disabled).toBe(true);
      expect(useOrbitStore().notice?.text).toContain(
        "配置目标或本地写入资格已变化",
      );
      expect(useOrbitStore().notice?.text).not.toContain(
        "synthetic-private-target",
      );
    },
  );

  it("requires a generated catalog even with synthetic verified runtime metadata", async () => {
    const value = preview();
    value.codex_catalog = null;
    value.codex_runtime = syntheticCodexRuntime();
    const view = show(value);
    const apply = view.get<HTMLButtonElement>(".button.primary");
    expect(apply.element.disabled).toBe(true);
    expect(view.text()).toContain("本次预览的执行来源已核验");
    expect(view.text()).toContain("0.163.0-alpha.10.2");
    await apply.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it("retires the preview when native rejects a late change to another catalog model", async () => {
    const store = useOrbitStore();
    store.models.push(syntheticCodexModel("synthetic-second"));
    const value = preview();
    value.codex_catalog!.aliases.push("synthetic-second");
    const view = show(value);
    store.models[1]!.revision = "synthetic-later-revision";
    store.models[1]!.codex_catalog_binding = {
      state: "unavailable",
      verified: false,
    };
    await nextTick();
    const apply = view.get<HTMLButtonElement>(".button.primary");
    // Native owns validation of the full catalog scope, beyond this dialog's primary model.
    expect(apply.element.disabled).toBe(false);
    bridge.call.mockRejectedValueOnce(
      new OrbitError("codex_catalog_scope_invalid", "synthetic-private-scope"),
    );
    await apply.trigger("click");
    await flushPromises();
    expect(bridge.call).toHaveBeenCalledWith("apply_client_config", {
      preview_id: value.id,
      confirmed: true,
    });
    expect(apply.element.disabled).toBe(true);
    expect(store.notice?.text).toContain("模型目录、权限或连接状态已变化");
    expect(store.notice?.text).not.toContain("synthetic-private-scope");
    await apply.trigger("click");
    expect(bridge.call).toHaveBeenCalledTimes(1);
  });

  it.each([
    [
      "claude_configuration_target_unverified",
      "尚未确认当前 Claude Desktop 版本的配置合同",
    ],
    [
      "claude_configuration_target_changed",
      "Claude Desktop 配置目标或本地写入资格已变化",
    ],
    [
      "claude_configuration_context_unverified",
      "Claude 原配置记录缺少可核验的恢复上下文",
    ],
    [
      "claude_configuration_context_changed",
      "Claude 配置的账户、节点、授权或文件状态已变化",
    ],
    [
      "claude_configuration_source_unsupported",
      "现有 Claude 配置来源无法安全迁移",
    ],
    ["codex_binding_unavailable", "当前模型缺少可用的 Codex 模板声明"],
    [
      "codex_configuration_target_unverified",
      "尚未确认已安装 Codex 的配置合同",
    ],
    ["codex_configuration_target_changed", "配置目标或本地写入资格已变化"],
    ["codex_reasoning_unavailable", "当前推理档位策略不足"],
    ["codex_catalog_template_unavailable", "当前模型目录无法安全生成"],
    ["codex_runtime_unverified", "尚未核验目标桌面应用的执行来源"],
    ["codex_runtime_changed", "目标桌面应用的执行来源已变化"],
    ["codex_catalog_scope_changed", "模型目录、权限或连接状态已变化"],
    ["codex_catalog_context_unverified", "缺少可核验的恢复上下文"],
    ["codex_config_override_unverified", "现有显式设置可能覆盖本次配置"],
  ])("explains blocked previews for %s", (code, explanation) => {
    const value = preview(code!.startsWith("claude_") ? "claude" : "codex");
    value.can_apply = false;
    value.blocking_reason = code!;
    value.codex_catalog = null;
    const view = show(value);
    expect(view.text()).toContain(explanation);
    expect(view.text()).not.toContain(code);
    expect(
      view.get<HTMLButtonElement>(".button.primary").element.disabled,
    ).toBe(true);
  });

  it("explains a native runtime rejection in English without exposing exception details", async () => {
    const state = bootstrap("synthetic-account-a", 2);
    state.settings.locale = "en";
    const store = useOrbitStore();
    store.acceptState(state);
    const view = show(preview());
    bridge.call.mockRejectedValueOnce(
      new OrbitError(
        "codex_runtime_changed",
        "synthetic-private-runtime-value",
      ),
    );
    const apply = view.get<HTMLButtonElement>(".button.primary");
    await apply.trigger("click");
    await flushPromises();
    expect(store.notice?.text).toContain("execution source changed");
    expect(store.notice?.text).not.toContain("synthetic-private-runtime-value");
    expect(store.notice?.text).not.toContain("codex_runtime_changed");
    expect(apply.element.disabled).toBe(true);
    await apply.trigger("click");
    expect(bridge.call).toHaveBeenCalledTimes(1);
  });
});
