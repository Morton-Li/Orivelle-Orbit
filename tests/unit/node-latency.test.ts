import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import { useNodeLatency } from "../../src/composables/useNodeLatency";
import { emptyState, useOrbitStore } from "../../src/stores/orbit";
import type { NodeLatencyView } from "../../src/shared/contracts";
import { bootstrap } from "../fixtures";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function result(
  milliseconds = 82,
  status: NodeLatencyView["status"] = "available",
): NodeLatencyView[] {
  return [
    {
      node_id: "node-a",
      node_revision: "node-revision-1",
      status,
      milliseconds: status === "available" ? milliseconds : null,
      measured_at: new Date().toISOString(),
    },
  ];
}
let wrapper: VueWrapper | undefined;
let latency: ReturnType<typeof useNodeLatency>;
let nextProbe: () => Promise<NodeLatencyView[]> | NodeLatencyView[];
let visibility: ReturnType<typeof vi.spyOn>;
const probeCalls = () =>
  bridge.call.mock.calls.filter(([command]) => command === "probe_nodes");

beforeEach(async () => {
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  vi.setSystemTime(new Date("2026-11-12T04:00:00Z"));
  visibility = vi
    .spyOn(document, "visibilityState", "get")
    .mockReturnValue("visible");
  setActivePinia(createPinia());
  nextProbe = () => result();
  bridge.call.mockReset().mockImplementation(async (command) => {
    if (command === "list_models") return [];
    if (command === "probe_nodes") return nextProbe();
    if (command === "cancel_node_probes") return null;
    throw new Error("Unexpected synthetic IPC");
  });
  useOrbitStore().acceptState(bootstrap());
  await flushPromises();
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
});
async function show() {
  wrapper = mount(
    defineComponent({
      setup() {
        latency = useNodeLatency();
        return () => h("div");
      },
    }),
  );
  await flushPromises();
}
function hide() {
  visibility.mockReturnValue("hidden");
  document.dispatchEvent(new Event("visibilitychange"));
}
function reveal() {
  visibility.mockReturnValue("visible");
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("node latency lifecycle", () => {
  it("measures on entry and every 30 seconds without changing the main connection state", async () => {
    const store = useOrbitStore();
    const before = JSON.stringify(store.state);
    await show();
    expect(latency.label(store.state.nodes[0]!)).toBe("82 ms");
    expect(probeCalls()).toHaveLength(1);
    expect(probeCalls()[0]?.[1]).toMatchObject({
      connection_id: store.activeConnection!.id,
      expected_revision: 1,
      node_ids: ["node-a"],
    });
    expect(probeCalls()[0]?.[1]).not.toHaveProperty("url");
    await vi.advanceTimersByTimeAsync(30_000);
    expect(probeCalls()).toHaveLength(2);
    expect(JSON.stringify(store.state)).toBe(before);
    expect(
      bridge.call.mock.calls.filter(([command]) => command === "list_models"),
    ).toHaveLength(1);
  });

  it("coalesces overlapping refreshes and retains the previous value while measuring", async () => {
    await show();
    const pending = deferred<NodeLatencyView[]>();
    nextProbe = () => pending.promise;
    const first = latency.refresh();
    const second = latency.refresh();
    expect(second).toBe(first);
    expect(latency.refreshing.value).toBe(true);
    expect(latency.label(useOrbitStore().state.nodes[0]!)).toBe("82 ms");
    await vi.advanceTimersByTimeAsync(60_000);
    expect(probeCalls()).toHaveLength(2);
    pending.resolve(result(95));
    await first;
    expect(latency.label(useOrbitStore().state.nodes[0]!)).toBe("95 ms");
    expect(latency.refreshing.value).toBe(false);
  });

  it("cancels hidden or unfocused work, marks old values stale and resumes once in the foreground", async () => {
    await show();
    const pending = deferred<NodeLatencyView[]>();
    nextProbe = () => pending.promise;
    void latency.refresh();
    hide();
    expect(
      bridge.call.mock.calls.filter(
        ([command]) => command === "cancel_node_probes",
      ),
    ).toHaveLength(1);
    pending.resolve(result(999));
    await flushPromises();
    await vi.advanceTimersByTimeAsync(90_000);
    expect(probeCalls()).toHaveLength(2);
    const resumed = deferred<NodeLatencyView[]>();
    nextProbe = () => resumed.promise;
    reveal();
    await flushPromises();
    expect(probeCalls()).toHaveLength(3);
    expect(latency.label(useOrbitStore().state.nodes[0]!)).toBe(
      "82 ms · 已过期",
    );
    resumed.resolve(result(73));
    await flushPromises();
    window.dispatchEvent(new Event("blur"));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(probeCalls()).toHaveLength(3);
    nextProbe = () => result(70);
    window.dispatchEvent(new Event("focus"));
    await flushPromises();
    expect(probeCalls()).toHaveLength(4);
    expect(latency.label(useOrbitStore().state.nodes[0]!)).toBe("70 ms");
  });

  it.each(["timeout", "failed"] as const)(
    "shows %s without changing service availability or surfacing native details",
    async (status) => {
      nextProbe = () => result(0, status);
      await show();
      const store = useOrbitStore();
      expect(latency.label(store.state.nodes[0]!)).toBe(
        status === "timeout" ? "探测超时" : "连接失败",
      );
      expect(store.state.nodes[0]!.available).toBe(true);
      expect(store.notice).toBeNull();
    },
  );

  it("drops a late old-account result even after returning to the same account", async () => {
    const first = deferred<NodeLatencyView[]>();
    nextProbe = () => first.promise;
    await show();
    nextProbe = () => result(32);
    useOrbitStore().acceptState(bootstrap("synthetic-account-b", 2));
    await flushPromises();
    expect(latency.measurements.value["node-a"]?.milliseconds).toBe(32);
    const fresh = deferred<NodeLatencyView[]>();
    nextProbe = () => fresh.promise;
    useOrbitStore().acceptState(bootstrap("synthetic-account-a", 3));
    await flushPromises();
    expect(latency.measurements.value).toEqual({});
    first.resolve(result(999));
    await flushPromises();
    expect(latency.measurements.value).toEqual({});
    fresh.resolve(result(52));
    await flushPromises();
    expect(latency.measurements.value["node-a"]?.milliseconds).toBe(52);
  });

  it("invalidates a changed endpoint and ignores results carrying an old node revision", async () => {
    await show();
    const pending = deferred<NodeLatencyView[]>();
    nextProbe = () => pending.promise;
    const changed = bootstrap("synthetic-account-a", 2);
    changed.nodes[0]!.base_url = "https://replacement.invalid/v1";
    changed.nodes[0]!.revision = "node-revision-2";
    useOrbitStore().acceptState(changed);
    await flushPromises();
    expect(latency.measurements.value).toEqual({});
    pending.resolve(result(999));
    await flushPromises();
    expect(latency.measurements.value).toEqual({});
  });

  it("clears account data on logout and stops all work on unmount", async () => {
    await show();
    const loggedOut = emptyState();
    loggedOut.state_revision = 2;
    useOrbitStore().acceptState(loggedOut);
    await flushPromises();
    expect(latency.measurements.value).toEqual({});
    await vi.advanceTimersByTimeAsync(60_000);
    expect(probeCalls()).toHaveLength(1);
    useOrbitStore().acceptState(bootstrap("synthetic-account-a", 3));
    await flushPromises();
    const pending = deferred<NodeLatencyView[]>();
    nextProbe = () => pending.promise;
    void latency.refresh();
    wrapper!.unmount();
    wrapper = undefined;
    const count = probeCalls().length;
    pending.resolve(result());
    await vi.advanceTimersByTimeAsync(90_000);
    expect(probeCalls()).toHaveLength(count);
    expect(
      bridge.call.mock.calls.some(
        ([command]) => command === "cancel_node_probes",
      ),
    ).toBe(true);
  });
});
