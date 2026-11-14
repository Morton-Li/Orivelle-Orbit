import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { BootstrapView } from "../../src/shared/contracts";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
  state: undefined as undefined | ((state: BootstrapView) => void),
  opened: undefined as undefined | (() => void),
  hidden: undefined as undefined | (() => void),
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  call: bridge.call,
  nativeAvailable: bridge.nativeAvailable,
  subscribeState: async (callback: (state: BootstrapView) => void) => {
    bridge.state = callback;
    return () => {};
  },
  subscribeMenubarOpened: async (callback: () => void) => {
    bridge.opened = callback;
    return () => {};
  },
  subscribeMenubarHidden: async (callback: () => void) => {
    bridge.hidden = callback;
    return () => {};
  },
  subscribeMenubarLayout: async () => () => {},
}));
import MenubarApp from "../../src/MenubarApp.vue";
import { bootstrap, task } from "../fixtures";
import { usageSummary } from "../usage-fixtures";
import { locale } from "../../src/shared/i18n";

let wrapper: VueWrapper | undefined;
let current: BootstrapView;
let osDark: boolean;
const mediaListeners = new Set<(event: { matches: boolean }) => void>();
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  vi.setSystemTime(new Date("2026-11-14T06:32:00Z"));
  osDark = false;
  mediaListeners.clear();
  vi.stubGlobal("matchMedia", () => ({
    matches: osDark,
    addEventListener: (_: string, fn: (event: { matches: boolean }) => void) =>
      mediaListeners.add(fn),
    removeEventListener: (
      _: string,
      fn: (event: { matches: boolean }) => void,
    ) => mediaListeners.delete(fn),
  }));
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  locale.value = "zh-CN";
  current = bootstrap();
  bridge.call.mockReset().mockImplementation(async (command, request) => {
    if (command === "bootstrap" || command === "refresh_account")
      return structuredClone(current);
    if (command === "read_usage_summary") return usageSummary(request);
    if (command === "menubar_control") {
      if (request.action === "ready") bridge.opened?.();
      return null;
    }
    throw new Error("Panel requested an unexpected service");
  });
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  document.documentElement.classList.remove("dark");
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
async function show() {
  wrapper = mount(MenubarApp);
  await flushPromises();
  return wrapper;
}
function changeOS(dark: boolean) {
  osDark = dark;
  mediaListeners.forEach((fn) => fn({ matches: dark }));
}
function push(state: BootstrapView) {
  current = state;
  bridge.state?.(structuredClone(state));
}

describe("macOS menu bar panel", () => {
  it("does not present queued work as already submitted", async () => {
    current.tasks = [task("queued")];
    await show();
    expect(wrapper!.get(".mb-stages").text()).toContain("提交中");
    expect(wrapper!.get(".mb-stages").text()).not.toContain("已提交");
    const waiting = structuredClone(current);
    waiting.state_revision++;
    waiting.tasks[0]!.state = "waiting";
    push(waiting);
    await flushPromises();
    expect(wrapper!.get(".mb-stages").text()).toContain("已提交");
  });
  it("follows Orbit overrides immediately and follows OS changes only in system mode", async () => {
    current.settings.theme = "light";
    await show();
    changeOS(true);
    await flushPromises();
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    const next = structuredClone(current);
    next.state_revision++;
    next.settings.theme = "dark";
    push(next);
    await flushPromises();
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    changeOS(false);
    await flushPromises();
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    const system = structuredClone(current);
    system.state_revision++;
    system.settings.theme = "system";
    push(system);
    await flushPromises();
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    changeOS(true);
    await flushPromises();
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("keeps currencies and pending money separate and does not start model or updater work", async () => {
    const data = usageSummary();
    data.summary.charges = [
      {
        currency: "USD",
        amount: null,
        known_amount: "9007199254740993.12345678",
        pending_count: 1,
      },
      { currency: "EUR", amount: "2.00", pending_count: 0 },
    ];
    bridge.call.mockImplementation(async (command, request) => {
      if (command === "bootstrap" || command === "refresh_account")
        return current;
      if (command === "read_usage_summary") return data;
      if (command === "menubar_control" && request.action === "ready")
        bridge.opened?.();
      return null;
    });
    await show();
    expect(wrapper!.get(".mb-spend").text()).toContain(
      "USD9,007,199,254,740,993.1235",
    );
    expect(wrapper!.get(".mb-spend").text()).toContain("EUR2.0000");
    expect(wrapper!.text()).toContain("笔调用等待完整费用记录");
    expect(
      bridge.call.mock.calls.some(([command]) =>
        ["list_models", "check_update", "probe_nodes"].includes(command),
      ),
    ).toBe(false);
  });

  it("discards late usage and task data from the previous account", async () => {
    let resolveOld!: (value: ReturnType<typeof usageSummary>) => void;
    current.tasks = [task("waiting")];
    bridge.call.mockImplementation(async (command, request) => {
      if (command === "bootstrap" || command === "refresh_account")
        return structuredClone(current);
      if (command === "menubar_control") {
        if (request.action === "ready") bridge.opened?.();
        return null;
      }
      if (command === "read_usage_summary")
        return request.connection_id.endsWith("account-a")
          ? new Promise((resolve) => {
              resolveOld = resolve;
            })
          : usageSummary(request);
      return null;
    });
    await show();
    push(bootstrap("synthetic-account-b", 10));
    await flushPromises();
    const old = usageSummary();
    old.summary.charges = [
      { currency: "USD", amount: "987654321.00", pending_count: 0 },
    ];
    resolveOld(old);
    await flushPromises();
    expect(wrapper!.text()).not.toContain("987,654,321");
    expect(wrapper!.text()).not.toContain("正在生成图片");
    expect(wrapper!.text()).toContain("synthetic-account-b");
  });

  it("stops hidden polling and exposes only typed navigation actions", async () => {
    await show();
    const before = bridge.call.mock.calls.filter(
      ([command]) => command === "read_usage_summary",
    ).length;
    bridge.hidden?.();
    await vi.advanceTimersByTimeAsync(120000);
    expect(
      bridge.call.mock.calls.filter(
        ([command]) => command === "read_usage_summary",
      ),
    ).toHaveLength(before);
    await wrapper!.get(".mb-open").trigger("click");
    expect(bridge.call).toHaveBeenCalledWith("menubar_control", {
      action: "open",
      destination: "home",
    });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(bridge.call).toHaveBeenCalledWith("menubar_control", {
      action: "hide",
    });
  });
});
