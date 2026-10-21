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
import { OrbitError } from "../../src/shared/bridge";
import type { ConfigPreview } from "../../src/shared/contracts";
import { useOrbitStore } from "../../src/stores/orbit";
import {
  bootstrap,
  syntheticCodexCatalog,
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
        : ["/fixture/config"],
    codex_catalog: client === "codex" ? syntheticCodexCatalog() : null,
    codex_runtime: client === "codex" ? syntheticCodexRuntime() : null,
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
    impact: [],
    redacted_diff: "synthetic config",
    expires_at: new Date(Date.now() + 60_000).toISOString(),
    can_apply: client === "codex",
    can_export_import: client === "claude",
    blocking_reason: client === "claude" ? "schema_unverified" : null,
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
  it.each(["yes", "unknown"] as const)(
    "retires an existing confirmation after managed becomes %s, including after it clears again",
    async (managed) => {
      const view = show(preview());
      const apply = view.get<HTMLButtonElement>(".button.primary");
      expect(apply.element.disabled).toBe(false);
      const changed = bootstrap("synthetic-account-a", 2);
      changed.clients[0]!.managed = managed;
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
        ...preview(),
        id: "22222222-2222-4222-8222-222222222222",
      };
      await view.setProps({ preview: fresh });
      expect(apply.element.disabled).toBe(false);
      bridge.call.mockImplementation(async (command) =>
        command === "bootstrap"
          ? bootstrap("synthetic-account-a", 4)
          : {
              client: "codex",
              state: "saved",
              loaded_state: "restart_required",
              backup_group_id: "synthetic-backup",
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

      await view.setProps({ preview: preview() });
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
    const view = show(preview("claude"));
    const exportButton = view
      .findAll<HTMLButtonElement>("button")
      .find((button) => button.text().includes("导出 Claude"))!;
    expect(exportButton.element.disabled).toBe(false);
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

describe("Codex catalog confirmation", () => {
  it("shows the compiled menu and explicit defaults separately from client loading and connection", () => {
    const value = preview();
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
    const context = view.findAll("dl")[1]!;
    expect(context.text()).toContain("默认模型调用名称synthetic-second");
    expect(context.text()).toContain("默认推理档位high");
    expect(context.text()).toContain("0.163.0-alpha.10.2");
    expect(view.text()).toContain("本次预览的执行来源已核验");
    expect(view.text()).toContain("不代表客户端已载入或连接通过");
    expect(view.text()).toContain("/fixture/config");
    expect(view.text()).toContain("/fixture/codex-model-catalog.json");
    expect(view.text()).not.toContain(value.codex_catalog!.source_sha256);
    expect(view.text()).not.toContain(value.codex_catalog!.sha256);
    expect(
      view.get<HTMLButtonElement>(".button.primary").element.disabled,
    ).toBe(false);
    expect(bridge.call).not.toHaveBeenCalled();
  });

  const unavailableRuntimes: {
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
  it.each(unavailableRuntimes)(
    "cannot apply a declared catalog when the execution source is $name",
    async ({ runtime, checked }) => {
      const value = preview();
      value.codex_runtime = runtime;
      const view = show(value);
      const apply = view.get<HTMLButtonElement>(".button.primary");
      expect(value.can_apply).toBe(true);
      expect(apply.element.disabled).toBe(true);
      expect(view.text()).toContain("执行来源尚未确认匹配声明目标");
      expect(view.text().includes("本次预览的执行来源已核验")).toBe(checked);
      await apply.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();
    },
  );

  it("requires a generated catalog even with synthetic verified runtime metadata", async () => {
    const value = preview();
    value.codex_catalog = null;
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
    ["codex_binding_unavailable", "当前模型缺少可用的 Codex 模板声明"],
    ["codex_reasoning_unavailable", "当前推理档位策略不足"],
    ["codex_catalog_template_unavailable", "当前模型目录无法安全生成"],
  ])("explains blocked previews for %s", (code, explanation) => {
    const value = preview();
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
});
