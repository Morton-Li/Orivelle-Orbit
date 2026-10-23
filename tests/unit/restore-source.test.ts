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
import type { BackupGroup, ConfigPreview } from "../../src/shared/contracts";
import { bootstrap, syntheticClaudeConfigurationTarget } from "../fixtures";

let view: VueWrapper | undefined;
function backup(id = "selected-backup"): BackupGroup {
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
function preview(source: BackupGroup): ConfigPreview {
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
      backup_group_id: source.id,
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
describe("Claude restore confirmation binds the selected backup", () => {
  it("retires the selected source when it disappears even though another same-owner backup remains", async () => {
    const selected = backup();
    const store = useOrbitStore();
    store.state.backups = [selected, backup("another-backup")];
    const button = show(preview(selected));
    expect(store.ready).toBe(false);
    expect(button.element.disabled).toBe(false);
    store.state.backups = [backup("another-backup")];
    await nextTick();
    expect(button.element.disabled).toBe(true);
    await button.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
    store.state.backups.push(selected);
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
    "permanently retires the confirmation when the selected backup's %s changes",
    async (change) => {
      const selected = backup();
      const store = useOrbitStore();
      store.state.backups = [selected, backup("another-backup")];
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
      store.state.backups[0] = changed;
      await nextTick();
      expect(button.element.disabled).toBe(true);
      store.state.backups[0] = selected;
      await nextTick();
      expect(button.element.disabled).toBe(true);
      await button.trigger("click");
      expect(bridge.call).not.toHaveBeenCalled();
    },
  );

  it("expires the selected source without a bootstrap refresh or a live login", async () => {
    const selected = backup();
    useOrbitStore().state.backups = [selected];
    const button = show(preview(selected));
    expect(button.element.disabled).toBe(false);
    vi.advanceTimersByTime(31_000);
    await nextTick();
    expect(button.element.disabled).toBe(true);
    expect(view!.text()).toContain("所选备份");
    await button.trigger("click");
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it.each([
    "missing source",
    "invalid deadline",
    "expired source",
    "invalid preview deadline",
  ] as const)("rejects a restore preview with %s", (fault) => {
    const selected = backup();
    useOrbitStore().state.backups = [selected];
    const value = preview(selected);
    if (value.operation !== "restore")
      throw new Error("Expected restore fixture");
    if (fault === "missing source")
      delete (value as Partial<typeof value>).restore_source;
    else if (fault === "invalid deadline")
      value.restore_source.expires_at = "invalid";
    else if (fault === "expired source") {
      value.restore_source.expires_at = new Date(
        Date.now() - 1000,
      ).toISOString();
      selected.expires_at = value.restore_source.expires_at;
    } else value.expires_at = "invalid";
    expect(show(value).element.disabled).toBe(true);
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it.each(["source", "owner", "client", "deadline", "file count"] as const)(
    "rejects a native restore response for a different %s than the selected row",
    async (change) => {
      const selected = backup();
      useOrbitStore().state.backups = [selected, backup("another-backup")];
      const result = preview(selected);
      if (result.operation !== "restore")
        throw new Error("Expected restore fixture");
      if (change === "source")
        result.restore_source.backup_group_id = "another-backup";
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
            ClientRow: true,
            ConfigReviewDialog: true,
            OrbitDialog: true,
          },
        },
      });
      await view
        .get("#backup-account")
        .setValue("https://panel.invalid/synthetic-account-a");
      await view.get(".backup-row .button.secondary").trigger("click");
      await flushPromises();
      expect(bridge.call).toHaveBeenCalledWith("preview_restore", {
        backup_group_id: selected.id,
      });
      expect(
        view.findComponent(ConfigReviewDialog).props("preview"),
      ).toBeNull();
      expect(useOrbitStore().notice?.text).toContain("所选备份");
    },
  );

  it("freezes the selected owner instead of adopting an in-place row mutation during preview", async () => {
    const selected = backup();
    const store = useOrbitStore();
    store.state.backups = [selected];
    let resolve!: (value: ConfigPreview) => void;
    bridge.call.mockReturnValueOnce(
      new Promise<ConfigPreview>((done) => {
        resolve = done;
      }),
    );
    view = mount(AccessPage, {
      global: {
        stubs: { ClientRow: true, ConfigReviewDialog: true, OrbitDialog: true },
      },
    });
    await view
      .get("#backup-account")
      .setValue("https://panel.invalid/synthetic-account-a");
    await view.get(".backup-row .button.secondary").trigger("click");
    store.state.backups[0]!.account_id = "another-account";
    resolve(preview(store.state.backups[0]!));
    await flushPromises();
    expect(view.findComponent(ConfigReviewDialog).props("preview")).toBeNull();
    expect(store.notice?.text).toContain("所选备份");
  });

  it("restores the explicitly selected backup while logged out without a model or inference request", async () => {
    const selected = backup();
    const store = useOrbitStore();
    store.state.backups = [selected, backup("another-backup")];
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
        backup_group_id: "completed-restore",
      })
      .mockResolvedValueOnce(refreshed);
    view = mount(AccessPage, {
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
    await view
      .get("#backup-account")
      .setValue("https://panel.invalid/synthetic-account-a");
    await view.get(".backup-row .button.secondary").trigger("click");
    await flushPromises();
    const confirm = view.get<HTMLButtonElement>(
      '[role="dialog"] .button.primary',
    );
    expect(confirm.element.disabled).toBe(false);
    await confirm.trigger("click");
    await flushPromises();
    expect(bridge.call.mock.calls).toEqual([
      ["preview_restore", { backup_group_id: selected.id }],
      [
        "restore_client_config",
        { preview_id: "restore-preview", confirmed: true },
      ],
      ["bootstrap", {}],
    ]);
    expect(store.notice?.text).toContain("原配置已恢复");
    expect(view.find('[role="dialog"]').exists()).toBe(false);
  });
});
