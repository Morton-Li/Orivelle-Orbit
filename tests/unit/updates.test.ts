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
import SettingsPage from "../../src/pages/SettingsPage.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import { OrbitError } from "../../src/shared/bridge";
import type { UpdateView } from "../../src/shared/contracts";
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
