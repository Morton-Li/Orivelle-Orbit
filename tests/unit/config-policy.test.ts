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
import type { ConfigPreview, ModelView } from "../../src/shared/contracts";
import { useOrbitStore } from "../../src/stores/orbit";
import { bootstrap, imageModel } from "../fixtures";

let wrapper: VueWrapper | undefined;
const model: ModelView = {
  ...imageModel("synthetic-text"),
  category: "text",
  capabilities: ["openai.responses", "anthropic.messages"],
};
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
    paths: ["/fixture/config"],
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
