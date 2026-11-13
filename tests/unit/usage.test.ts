import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import UsagePage from "../../src/pages/UsagePage.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import { bootstrap, orbitSelectStub } from "../fixtures";
import { usageSummary, usagePage } from "../usage-fixtures";
import { validUsageRange } from "../../src/shared/usage";
import { locale } from "../../src/shared/i18n";
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  locale.value = "zh-CN";
  setActivePinia(createPinia());
  bridge.call
    .mockReset()
    .mockImplementation((command, query) =>
      command === "read_usage_summary"
        ? usageSummary(query)
        : command === "list_usage_records"
          ? usagePage(query)
          : [],
    );
  const state = bootstrap();
  state.connections[0]!.permission_state = "readonly";
  state.session.inference_credential_status = "permission_denied";
  useOrbitStore().acceptState(state);
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});
function show() {
  wrapper = mount(UsagePage, {
    global: { stubs: { OrbitSelect: orbitSelectStub } },
  });
  return wrapper;
}
describe("platform usage", () => {
  it("keeps large decimal amounts and different currencies separate", async () => {
    const data = usageSummary();
    data.summary.charges = [
      {
        currency: "EUR",
        amount: "2.00000001",
        known_amount: "2.00000001",
        pending_count: 0,
      },
      {
        currency: "USD",
        amount: null,
        known_amount: "9007199254740993.12345678",
        pending_count: 1,
      },
    ];
    bridge.call.mockImplementation((command, query) =>
      command === "read_usage_summary"
        ? data
        : command === "list_usage_records"
          ? usagePage(query)
          : [],
    );
    show();
    await flushPromises();
    const card = wrapper!.get(".usage-stat-accent");
    expect(card.findAll("strong")).toHaveLength(2);
    expect(card.text()).toContain("EUR 2.0000");
    expect(card.text()).toContain("USD 9,007,199,254,740,993.1235");
  });

  it("lets read-only accounts query usage without a node or inference credential and preserves pending and exact money", async () => {
    useOrbitStore().activeConnection!.node_id = null;
    show();
    await flushPromises();
    expect(wrapper!.findAll(".usage-stat")).toHaveLength(4);
    expect(wrapper!.text()).toContain("当前账户 · 全部平台入口");
    expect(wrapper!.text()).toContain("费用待确认");
    expect(bridge.call).toHaveBeenCalledWith(
      "read_usage_summary",
      expect.objectContaining({
        connection_id: "connection-synthetic-account-a",
      }),
    );
    expect(wrapper!.findAll(".usage-table tbody tr").length).toBeGreaterThan(0);
    const table = wrapper!.get(".usage-table");
    expect(table.text()).toContain("USD 0.0182");
    expect(table.text()).toContain("费用待确认");
  });
  it("keeps valid records visible after a load failure and resets cursors when filtering", async () => {
    show();
    await flushPromises();
    await wrapper!.get("#usage-outcome").setValue("failed");
    await flushPromises();
    expect(bridge.call).toHaveBeenLastCalledWith(
      "list_usage_records",
      expect.objectContaining({ outcome: "failed" }),
    );
    const before = wrapper!.get(".usage-table tbody").text();
    bridge.call.mockRejectedValue(new Error("private native body"));
    const refresh = wrapper!
      .findAll("button")
      .find((b) => b.text() === "刷新用量")!;
    await refresh.trigger("click");
    await flushPromises();
    expect(wrapper!.get(".usage-table tbody").text()).toBe(before);
    expect(wrapper!.text()).toContain("加载未完成");
    expect(wrapper!.text()).not.toContain("private native body");
  });
  it("discards late data from an account that has been switched away from", async () => {
    let done!: (value: ReturnType<typeof usageSummary>) => void;
    bridge.call.mockImplementation((command, query) =>
      command === "read_usage_summary"
        ? new Promise((resolve) => {
            done = resolve;
          })
        : command === "list_usage_records"
          ? usagePage(query)
          : [],
    );
    show();
    const old = done;
    useOrbitStore().acceptState(bootstrap("synthetic-account-b", 2));
    const data = usageSummary();
    data.summary.requests = 987654321;
    old(data);
    await flushPromises();
    expect(wrapper!.text()).not.toContain("987,654,321");
    expect(wrapper!.findAll(".usage-stat")[0]!.text()).toContain("—");
  });
  it("validates calendar dates and the 90-day read boundary", () => {
    expect(validUsageRange("2026-02-30", "2026-03-01")).toBe(false);
    expect(validUsageRange("2026-01-01", "2026-04-01")).toBe(false);
    expect(validUsageRange("2026-01-01", "2026-03-31")).toBe(true);
  });
});
