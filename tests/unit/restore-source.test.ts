import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import ConfigReviewDialog from "../../src/components/ConfigReviewDialog.vue";
import AccessPage from "../../src/pages/AccessPage.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import type { SwitchSnapshot, ConfigPreview } from "../../src/shared/contracts";
import {
  bootstrap,
  orbitSelectStub,
  syntheticClaudeConfigurationTarget,
} from "../fixtures";

let view: VueWrapper | undefined;
function snapshot(id = "selected-snapshot"): SwitchSnapshot {
  return {
    id,
    client: "claude",
    account_id: "synthetic-account-a",
    account_label: "Original account",
    panel_origin: "https://panel.invalid",
    installation_id: "installation-a",
    created_at: "2026-10-22T00:00:00Z",
    expires_at: new Date(Date.now() + 30_000).toISOString(),
    file_count: 3,
    state: "available",
  };
}
function preview(source: SwitchSnapshot): ConfigPreview {
  return {
    id: "restore-preview",
    client: "claude",
    operation: "restore",
    target_version: "synthetic-version",
    paths: [
      "/fixture/main.json",
      "/fixture/_meta.json",
      "/fixture/new-profile.json",
    ],
    model_id: null,
    context: null,
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
    impact: [],
    redacted_diff: "Encrypted originals remain redacted",
    expires_at: new Date(Date.now() + 60_000).toISOString(),
    can_apply: true,
    can_export_import: false,
    blocking_reason: null,
    claude_configuration_target: syntheticClaudeConfigurationTarget(),
    codex_configuration_target: null,
    codex_catalog: null,
    codex_runtime: null,
  };
}
beforeEach(() => {
  vi.useFakeTimers();
  setActivePinia(createPinia());
  bridge.call.mockReset().mockResolvedValue([]);
  const state = bootstrap();
  state.session.status = "logged_out";
  state.session.account = null;
  state.session.inference_credential_status = "unavailable";
  state.clients = state.clients.filter((client) => client.client === "claude");
  useOrbitStore().acceptState(state);
});
afterEach(() => {
  view?.unmount();
  view = undefined;
  vi.useRealTimers();
});
function show(value: ConfigPreview) {
  view = mount(ConfigReviewDialog, {
    props: { preview: value },
    global: { stubs: { OrbitDialog: { template: "<div><slot /></div>" } } },
  });
  return view.get<HTMLButtonElement>(".button.primary");
}
describe("Claude restore confirmation binds the selected snapshot", () => {
  it("retires the selected source when it disappears even though another same-owner snapshot remains", async () => {
    const selected = snapshot();
    const store = useOrbitStore();
    store.state.switch_snapshots = [selected, snapshot("another-snapshot")];
    const button = show(preview(selected));
    expect(store.ready).toBe(false);
    expect(button.element.disabled).toBe(false);
    store.state.switch_snapshots = [snapshot("another-snapshot")];
    await nextTick();
    expect(button.element.disabled).toBe(true);
    await button.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
    store.state.switch_snapshots.push(selected);
    await nextTick();
    expect(button.element.disabled).toBe(true);
  });

  it.each([
    "owner",
    "installation",
    "origin",
    "client",
    "file count",
    "deadline",
    "unavailable",
  ] as const)(
    "permanently retires the confirmation when the selected snapshot's %s changes",
    async (change) => {
      const selected = snapshot();
      const store = useOrbitStore();
      store.state.switch_snapshots = [selected, snapshot("another-snapshot")];
      const button = show(preview(selected));
      expect(button.element.disabled).toBe(false);
      const changed = { ...selected };
      if (change === "owner") changed.account_id = "other-actor";
      else if (change === "installation")
        changed.installation_id = "other-installation";
      else if (change === "origin")
        changed.panel_origin = "https://other.invalid";
      else if (change === "client") changed.client = "codex";
      else if (change === "file count") changed.file_count = 2;
      else if (change === "deadline")
        changed.expires_at = new Date(Date.now() + 45_000).toISOString();
      else changed.state = "unavailable";
      store.state.switch_snapshots[0] = changed;
      await nextTick();
      expect(button.element.disabled).toBe(true);
      store.state.switch_snapshots[0] = selected;
      await nextTick();
      expect(button.element.disabled).toBe(true);
      await button.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();
    },
  );

  it("expires the preview without a bootstrap refresh or a live login", async () => {
    const selected = snapshot();
    useOrbitStore().state.switch_snapshots = [selected];
    const button = show(preview(selected));
    expect(button.element.disabled).toBe(false);
    vi.advanceTimersByTime(61_000);
    await nextTick();
    expect(button.element.disabled).toBe(true);
    expect(view!.text()).toContain("原配置记录");
    await button.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it.each([
    "missing source",
    "invalid deadline",
    "mismatched source deadline",
    "invalid preview deadline",
  ] as const)("rejects a restore preview with %s", (fault) => {
    const selected = snapshot();
    useOrbitStore().state.switch_snapshots = [selected];
    const value = preview(selected);
    if (value.operation !== "restore")
      throw new Error("Expected restore fixture");
    if (fault === "missing source")
      delete (value as Partial<typeof value>).restore_source;
    else if (fault === "invalid deadline")
      value.restore_source.expires_at = "invalid";
    else if (fault === "mismatched source deadline") {
      value.restore_source.expires_at = new Date(
        Date.now() - 1000,
      ).toISOString();
    } else value.expires_at = "invalid";
    expect(show(value).element.disabled).toBe(true);
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it.each(["source", "owner", "client", "deadline", "file count"] as const)(
    "rejects a native restore response for a different %s than the selected row",
    async (change) => {
      const selected = snapshot();
      useOrbitStore().state.switch_snapshots = [
        selected,
        snapshot("another-snapshot"),
      ];
      const result = preview(selected);
      if (result.operation !== "restore")
        throw new Error("Expected restore fixture");
      if (change === "source")
        result.restore_source.snapshot_id = "another-snapshot";
      else if (change === "owner")
        result.restore_owner.account_id = "another-account";
      else if (change === "client") result.client = "codex";
      else if (change === "deadline")
        result.restore_source.expires_at = new Date(
          Date.now() + 45_000,
        ).toISOString();
      else result.paths.pop();
      bridge.call.mockResolvedValueOnce(result);
      view = mount(AccessPage, {
        global: {
          stubs: {
            OrbitSelect: orbitSelectStub,
            ClientRow: true,
            ConfigReviewDialog: true,
            OrbitDialog: true,
          },
        },
      });
      await view.get("#client-claude .button.secondary").trigger("click");
      await flushPromises();
      expect(bridge.call).toHaveBeenCalledWith("preview_restore", {
        snapshot_id: selected.id,
      });
      expect(
        view.findComponent(ConfigReviewDialog).props("preview"),
      ).toBeNull();
      expect(useOrbitStore().notice?.text).toContain("原配置记录");
    },
  );

  it("freezes the selected owner instead of adopting an in-place row mutation during preview", async () => {
    const selected = snapshot();
    const store = useOrbitStore();
    store.state.switch_snapshots = [selected];
    let resolve!: (value: ConfigPreview) => void;
    bridge.call.mockReturnValueOnce(
      new Promise<ConfigPreview>((done) => {
        resolve = done;
      }),
    );
    view = mount(AccessPage, {
      global: {
        stubs: {
          OrbitSelect: orbitSelectStub,
          ClientRow: true,
          ConfigReviewDialog: true,
          OrbitDialog: true,
        },
      },
    });
    await view.get("#client-claude .button.secondary").trigger("click");
    store.state.switch_snapshots[0]!.account_id = "another-account";
    resolve(preview(store.state.switch_snapshots[0]!));
    await flushPromises();
    expect(view.findComponent(ConfigReviewDialog).props("preview")).toBeNull();
    expect(store.notice?.text).toContain("原配置记录");
  });

  it("restores the explicitly selected snapshot while logged out without a model or inference request", async () => {
    const selected = snapshot();
    const store = useOrbitStore();
    store.state.switch_snapshots = [selected, snapshot("another-snapshot")];
    const refreshed = {
      ...store.state,
      state_revision: store.state.state_revision + 1,
    };
    bridge.call
      .mockResolvedValueOnce(preview(selected))
      .mockResolvedValueOnce({
        client: "claude",
        state: "restored",
        loaded_state: "restart_required",
        snapshot_id: "completed-restore",
      })
      .mockResolvedValueOnce(refreshed);
    view = mount(AccessPage, {
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
    await view.get("#client-claude .button.secondary").trigger("click");
    await flushPromises();
    const confirm = view.get<HTMLButtonElement>(
      '[role="dialog"] .button.primary',
    );
    expect(confirm.element.disabled).toBe(false);
    await confirm.trigger("click");
    await flushPromises();
    expect(bridge.call.mock.calls).toEqual([
      ["preview_restore", { snapshot_id: selected.id }],
      [
        "restore_client_config",
        { preview_id: "restore-preview", confirmed: true },
      ],
      ["bootstrap", {}],
    ]);
    expect(store.notice?.text).toContain("原配置已恢复");
    expect(view.find('[role="dialog"]').exists()).toBe(false);
  });

  it("only previews preference coordination after its separate explicit action", async () => {
    const selected = snapshot();
    useOrbitStore().state.switch_snapshots = [selected];
    const result = Object.assign(preview(selected), {
      restore_coordination: "claude_main_preferences_v1" as const,
    });
    bridge.call.mockResolvedValueOnce(result);
    view = mount(AccessPage, {
      global: {
        stubs: {
          OrbitSelect: orbitSelectStub,
          ClientRow: true,
          ConfigReviewDialog: true,
          OrbitDialog: true,
        },
      },
    });
    const action = view
      .findAll("#client-claude button")
      .find((button) => button.text() === "保留 Claude 偏好后切回");
    expect(
      action,
      "separate preference coordination preview action",
    ).toBeDefined();
    await action!.trigger("click");
    await flushPromises();
    expect(bridge.call.mock.calls).toEqual([
      [
        "preview_restore",
        {
          snapshot_id: selected.id,
          coordination: "claude_main_preferences_v1",
        },
      ],
    ]);
    expect(view.findComponent(ConfigReviewDialog).props("preview")).toEqual(
      result,
    );
  });

  it.each([
    ["coordinated request", true, undefined],
    ["strict request", false, "claude_main_preferences_v1"],
    ["unknown coordination", true, "unknown_coordination"],
  ] as const)(
    "rejects a mismatched native mode for %s",
    async (_description, coordinated, nativeMode) => {
      const selected = snapshot();
      useOrbitStore().state.switch_snapshots = [selected];
      bridge.call.mockResolvedValueOnce(
        Object.assign(preview(selected), { restore_coordination: nativeMode }),
      );
      view = mount(AccessPage, {
        global: {
          stubs: {
            OrbitSelect: orbitSelectStub,
            ClientRow: true,
            ConfigReviewDialog: true,
            OrbitDialog: true,
          },
        },
      });
      const action = coordinated
        ? view
            .findAll("#client-claude button")
            .find((button) => button.text() === "保留 Claude 偏好后切回")
        : view.get("#client-claude .button.secondary");
      expect(action).toBeDefined();
      await action!.trigger("click");
      await flushPromises();
      expect(
        view.findComponent(ConfigReviewDialog).props("preview"),
      ).toBeNull();
      expect(useOrbitStore().notice?.text).toContain("原配置记录");
      expect(bridge.call.mock.calls).toHaveLength(1);
    },
  );

  it("requires the explicit preserving-preferences confirmation without sending file content", async () => {
    const selected = snapshot();
    const store = useOrbitStore();
    store.state.switch_snapshots = [selected];
    const value = Object.assign(preview(selected), {
      restore_coordination: "claude_main_preferences_v1" as const,
      impact: ["保留后续偏好，只恢复原接入模式。"],
      redacted_diff: "Preference contents stay redacted.",
    });
    const refreshed = {
      ...store.state,
      state_revision: store.state.state_revision + 1,
    };
    bridge.call
      .mockResolvedValueOnce({
        client: "claude",
        state: "restored",
        loaded_state: "restart_required",
        snapshot_id: "coordinated-restore",
      })
      .mockResolvedValueOnce(refreshed);
    const button = show(value);
    expect(button.text()).toBe("确认保留偏好并恢复接入");
    expect(button.element.disabled).toBe(false);
    expect(bridge.call).not.toHaveBeenCalled();
    await button.trigger("click");
    await flushPromises();
    expect(bridge.call.mock.calls).toEqual([
      ["restore_client_config", { preview_id: value.id, confirmed: true }],
      ["bootstrap", {}],
    ]);
    expect(store.notice?.text).toContain("接入配置已恢复");
  });

  it("disables confirmation for an unknown native coordination mode", () => {
    const selected = snapshot();
    useOrbitStore().state.switch_snapshots = [selected];
    const value = Object.assign(preview(selected), {
      restore_coordination: "unknown_coordination",
    });
    const button = show(value);
    expect(button.element.disabled).toBe(true);
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it("does not offer preference coordination for another client or an unavailable snapshot", async () => {
    const other = { ...snapshot("other-client"), client: "codex" as const };
    const unavailable = { ...snapshot(), state: "unavailable" as const };
    useOrbitStore().state.switch_snapshots = [other, unavailable];
    view = mount(AccessPage, {
      global: {
        stubs: {
          OrbitSelect: orbitSelectStub,
          ClientRow: true,
          ConfigReviewDialog: true,
          OrbitDialog: true,
        },
      },
    });
    expect(view.find("#client-codex .text-button").exists()).toBe(false);
    expect(
      view.get<HTMLButtonElement>("#client-claude .text-button").element
        .disabled,
    ).toBe(true);
    expect(bridge.call).not.toHaveBeenCalled();
  });
});
