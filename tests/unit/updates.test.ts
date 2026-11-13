import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
  subscribeUpdateProgress: vi.fn(),
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import SettingsPage from "../../src/pages/SettingsPage.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import { OrbitError } from "../../src/shared/bridge";
import type { UpdateProgress, UpdateView } from "../../src/shared/contracts";
import { bootstrap, orbitSelectStub } from "../fixtures";

let view: VueWrapper | undefined;
function state(supported = true, automatic = false) {
  const value = bootstrap();
  value.session.status = "logged_out";
  value.session.account = null;
  value.session.inference_credential_status = "unavailable";
  value.in_app_updates = supported;
  value.settings.check_updates_on_start = automatic;
  return value;
}
function latest(): UpdateView {
  return {
    status: "up_to_date",
    current_version: "0.1.0",
    version: null,
    reason: null,
  };
}
function show() {
  view = mount(SettingsPage, {
    global: { stubs: { OrbitSelect: orbitSelectStub, OrbitDialog: true } },
  });
  return view;
}
beforeEach(() => {
  setActivePinia(createPinia());
  bridge.call.mockReset();
  bridge.subscribeUpdateProgress.mockReset();
  bridge.subscribeUpdateProgress.mockResolvedValue(() => undefined);
});
afterEach(() => {
  view?.unmount();
  view = undefined;
  useOrbitStore().dispose();
});

describe("update capability and lifecycle", () => {
  it("does not mistake an unchecked supported build for an unconfigured channel", () => {
    useOrbitStore().acceptState(state());
    const page = show();
    expect(page.get(".update-status").text()).toBe("尚未检查更新");
    expect(page.text()).toContain("启动时检查更新");
    expect(page.text()).not.toContain("发布渠道尚未配置");
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it("hides unavailable controls and explains manual installation", () => {
    useOrbitStore().acceptState(state(false, true));
    const page = show();
    expect(page.get(".update-status").text()).toBe(
      "当前版本需手动下载安装包更新",
    );
    expect(page.text()).not.toContain("启动时检查更新");
    expect(
      page.findAll("button").some((button) => button.text() === "检查更新"),
    ).toBe(false);
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it("shows checking during the request, prevents duplicate checks and keeps the result across page visits", async () => {
    const store = useOrbitStore();
    store.acceptState(state());
    let resolve!: (result: UpdateView) => void;
    bridge.call.mockReturnValue(
      new Promise<UpdateView>((done) => {
        resolve = done;
      }),
    );
    const request = store.checkUpdates();
    const page = show();
    await nextTick();
    expect(page.get(".update-status").text()).toBe("正在检查更新…");
    await store.checkUpdates();
    expect(bridge.call.mock.calls).toEqual([["check_update", {}]]);
    resolve(latest());
    await request;
    await nextTick();
    expect(page.get(".update-status").text()).toBe("当前已是最新版本");
    expect(store.notice).toEqual({ text: "当前已是最新版本", tone: "success" });
    view!.unmount();
    expect(show().get(".update-status").text()).toBe("当前已是最新版本");
  });

  it.each([
    [true, true, 1],
    [true, false, 0],
    [false, true, 0],
  ] as const)(
    "checks once at startup only when supported=%s and enabled=%s",
    async (supported, automatic, count) => {
      bridge.call.mockImplementation(async (command) => {
        if (command === "bootstrap") return state(supported, automatic);
        if (command === "check_update") return latest();
        throw new Error(`Unexpected command: ${command}`);
      });
      const store = useOrbitStore();
      await Promise.all([store.initialize(), store.initialize()]);
      await flushPromises();
      expect(
        bridge.call.mock.calls.filter(
          ([command]) => command === "check_update",
        ),
      ).toHaveLength(count);
      expect(
        bridge.call.mock.calls.every(([command]) =>
          ["bootstrap", "check_update"].includes(command),
        ),
      ).toBe(true);
      if (count)
        expect(show().get(".update-status").text()).toBe("当前已是最新版本");
    },
  );

  it("keeps a failed startup check visible without claiming success or displaying an unsolicited error toast", async () => {
    bridge.call.mockImplementation(async (command) => {
      if (command === "bootstrap") return state(true, true);
      throw new OrbitError(
        "update_channel_unavailable",
        "Synthetic network failure",
      );
    });
    const store = useOrbitStore();
    await store.initialize();
    await flushPromises();
    expect(show().get(".update-status").text()).toBe("检查更新失败，请重试");
    expect(store.notice).toBeNull();
    bridge.call.mockResolvedValue(latest());
    await store.checkUpdates();
    await nextTick();
    expect(view!.get(".update-status").text()).toBe("当前已是最新版本");
  });

  it("switches to manual installation when native checking reports unavailable capability", async () => {
    const store = useOrbitStore();
    store.acceptState(state());
    bridge.call.mockResolvedValue({
      ...latest(),
      status: "disabled",
      reason: "internal build details",
    });
    await store.checkUpdates();
    const page = show();
    expect(page.get(".update-status").text()).toBe(
      "当前版本需手动下载安装包更新",
    );
    expect(page.text()).not.toContain("internal build details");
    expect(page.text()).not.toContain("启动时检查更新");
  });
});

function available(): UpdateView {
  return { ...latest(), status: "available", version: "0.2.0" };
}
function button(page: VueWrapper, label: string) {
  return page.findAll("button").find((button) => button.text() === label)!;
}
describe("update downloads", () => {
  it("announces a new version, shows native progress and opens install confirmation after download", async () => {
    const store = useOrbitStore();
    store.acceptState(state());
    bridge.call.mockResolvedValueOnce(available());
    await store.checkUpdates();
    expect(store.notice?.text).toBe("发现新版本 · 0.2.0");
    let progress!: (progress: UpdateProgress) => void;
    const stop = vi.fn();
    bridge.subscribeUpdateProgress.mockImplementation(async (callback) => {
      progress = callback;
      return stop;
    });
    let finish!: (result: UpdateView) => void;
    bridge.call.mockImplementation(
      () =>
        new Promise<UpdateView>((resolve) => {
          finish = resolve;
        }),
    );
    const page = show();
    await button(page, "下载并更新").trigger("click");
    await flushPromises();
    const request = bridge.call.mock.calls.at(-1)![1];
    expect(bridge.subscribeUpdateProgress).toHaveBeenCalledOnce();
    expect(button(page, "取消下载").attributes("disabled")).toBeUndefined();
    expect(page.get("progress").attributes("value")).toBeUndefined();
    expect(button(page, "下载并更新")).toBeUndefined();
    progress({
      request_id: "stale-download",
      phase: "downloading",
      downloaded_bytes: 90,
      total_bytes: 100,
    });
    expect(store.updateProgress?.downloaded_bytes).toBe(0);
    progress({
      request_id: request.request_id,
      phase: "downloading",
      downloaded_bytes: 50,
      total_bytes: 100,
    });
    await nextTick();
    expect(page.get("progress").attributes("value")).toBe("50");
    expect(page.text()).toContain("50%");
    progress({
      request_id: request.request_id,
      phase: "preparing",
      downloaded_bytes: 100,
      total_bytes: 100,
    });
    await nextTick();
    expect(page.get(".update-status").text()).toBe("下载完成，正在准备安装…");
    finish({ ...available(), status: "verified" });
    await flushPromises();
    const dialog = page.findAllComponents({ name: "OrbitDialog" }).at(-1)!;
    expect(dialog.props("open")).toBe(true);
    expect(dialog.props("title")).toBe("安装并重启");
    expect(
      bridge.call.mock.calls.filter(
        ([command]) => command === "install_update",
      ),
    ).toHaveLength(0);
    expect(store.busy.has("update")).toBe(false);
    expect(stop).toHaveBeenCalledOnce();
  });

  it("cancels the native attempt, allows retry and ignores late events from the old download", async () => {
    const store = useOrbitStore();
    store.acceptState(state());
    store.update = available();
    let fail!: (error: OrbitError) => void;
    let currentId = "";
    const callbacks: ((progress: UpdateProgress) => void)[] = [];
    bridge.subscribeUpdateProgress.mockImplementation(async (callback) => {
      callbacks.push(callback);
      return () => undefined;
    });
    bridge.call.mockImplementation(async (command, request) => {
      if (command === "cancel_update_download") {
        expect(request.request_id).toBe(currentId);
        fail(new OrbitError("update_download_cancelled", "native details"));
        return null;
      }
      currentId = request.request_id;
      return new Promise<UpdateView>((_, reject) => {
        fail = reject;
      });
    });
    const download = store.downloadUpdate();
    await flushPromises();
    const oldId = currentId;
    await store.downloadUpdate();
    expect(bridge.call).toHaveBeenCalledOnce();
    await store.cancelUpdateDownload();
    await download;
    expect(store.notice).toEqual({
      text: "已取消下载，可随时重新下载",
      tone: "neutral",
    });
    expect(store.update?.status).toBe("available");
    expect(store.busy.has("update")).toBe(false);
    const retry = store.downloadUpdate();
    await flushPromises();
    expect(currentId).not.toBe(oldId);
    callbacks[0]!({
      request_id: oldId,
      phase: "downloading",
      downloaded_bytes: 80,
      total_bytes: 100,
    });
    expect(store.updateProgress?.downloaded_bytes).toBe(0);
    fail(new OrbitError("update_request_timeout", "internal URL"));
    await retry;
  });

  it.each([
    "result_unconfirmed",
    "update_request_timeout",
    "update_request_failed",
    "invalid_update_signature",
  ])(
    "uses a readable update error for %s and restores retry controls",
    async (code) => {
      const store = useOrbitStore();
      store.acceptState(state());
      store.update = available();
      bridge.call.mockRejectedValue(
        new OrbitError(code, "取消本地等待不能保证服务端停止或不结算。"),
      );
      const page = show();
      await button(page, "下载并更新").trigger("click");
      await flushPromises();
      expect(store.notice?.text).not.toMatch(
        /不结算|result_unconfirmed|internal|\(/,
      );
      expect(store.notice?.text).toMatch(/重新下载/);
      expect(button(page, "检查更新").attributes("disabled")).toBeUndefined();
      expect(button(page, "下载并更新").attributes("disabled")).toBeUndefined();
      expect(page.find("progress").exists()).toBe(false);
    },
  );

  it("allows cancellation before the progress listener has finished registering", async () => {
    const store = useOrbitStore();
    store.acceptState(state());
    store.update = available();
    let ready!: (stop: () => void) => void;
    bridge.subscribeUpdateProgress.mockReturnValue(
      new Promise((resolve) => {
        ready = resolve;
      }),
    );
    bridge.call.mockImplementation(async (command) => {
      if (command === "cancel_update_download") return null;
      throw new OrbitError("update_download_cancelled", "");
    });
    const download = store.downloadUpdate();
    await store.cancelUpdateDownload();
    expect(bridge.call.mock.calls[0]![0]).toBe("cancel_update_download");
    ready(() => undefined);
    await download;
    expect(store.busy.has("update")).toBe(false);
  });
});
