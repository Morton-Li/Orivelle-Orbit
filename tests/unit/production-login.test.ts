import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";

const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
  subscribeState: vi.fn(async () => () => undefined),
  subscribeErrors: vi.fn(async () => () => undefined),
  subscribeAuthorization: vi.fn(async () => () => undefined),
}));
vi.mock("../../src/shared/bridge", async (original) => ({
  ...(await original<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import App from "../../src/App.vue";
import { useOrbitStore } from "../../src/stores/orbit";
import { bootstrap } from "../fixtures";

let view: VueWrapper | undefined;
beforeEach(() => {
  setActivePinia(createPinia());
  bridge.call.mockReset();
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
});
afterEach(() => {
  view?.unmount();
  vi.unstubAllGlobals();
});

it("connects only to the fixed official service despite a legacy connection origin", async () => {
  const state = bootstrap();
  state.session.status = "expired";
  state.session.account = null;
  state.connections[0]!.panel_origin = "https://legacy-panel.invalid";
  bridge.call.mockImplementation(async (command) => {
    if (command === "bootstrap") return state;
    if (command === "start_login")
      return {
        request_id: "official-login",
        expires_at: new Date(Date.now() + 60_000).toISOString(),
      };
    return [];
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ["/", "/images", "/access", "/settings"].map((path) => ({
      path,
      component: { template: "<div />" },
    })),
  });
  await router.push("/");
  await router.isReady();
  view = mount(App, {
    global: {
      plugins: [router],
      stubs: {
        OrbitDialog: {
          props: ["open"],
          template: '<section v-if="open" role="dialog"><slot /></section>',
        },
        OrbitIcon: true,
      },
    },
  });
  await flushPromises();
  useOrbitStore().loginOpen = true;
  await flushPromises();
  const dialog = view.get('[role="dialog"]');
  expect(dialog.text()).not.toContain("https://orivelle.powerbyte.io");
  expect(dialog.text()).not.toContain("legacy-panel.invalid");
  expect(dialog.findAll("input, textarea, select")).toHaveLength(0);
  await dialog.get("form").trigger("submit");
  await flushPromises();
  expect(bridge.call).toHaveBeenCalledWith("start_login", {});
  expect(useOrbitStore().loginPending).toBe(true);
});
