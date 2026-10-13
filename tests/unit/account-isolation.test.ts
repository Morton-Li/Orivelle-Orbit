import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
const bridge = vi.hoisted(() => ({
  call: vi.fn(),
  nativeAvailable: () => true,
  subscribeState: vi.fn(async () => () => undefined),
  subscribeErrors: vi.fn(async () => () => undefined),
  subscribeAuthorization: vi.fn(async () => () => undefined),
}));
vi.mock("../../src/shared/bridge", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../src/shared/bridge")>()),
  ...bridge,
}));
import { useOrbitStore } from "../../src/stores/orbit";
import { bootstrap, imageModel, task } from "../fixtures";
import type { ModelView } from "../../src/shared/contracts";
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
beforeEach(() => {
  setActivePinia(createPinia());
  bridge.call.mockReset();
  bridge.call.mockResolvedValue([]);
});
describe("account and revision isolation", () => {
  it("blocks mutations during update maintenance while allowing a local state read", async () => {
    const store = useOrbitStore();
    const state = bootstrap();
    state.maintenance_active = true;
    bridge.call.mockResolvedValue(state);
    store.acceptState(state);
    await Promise.resolve();
    bridge.call.mockClear();
    expect(store.ready).toBe(false);
    await store.perform("restore", "restore_client_config", {
      preview_id: "fixture",
      confirmed: true,
    });
    await store.perform("settings", "update_settings", {
      patch: { locale: "en" },
    });
    expect(bridge.call).not.toHaveBeenCalled();
    await store.refresh();
    expect(bridge.call).toHaveBeenCalledWith("bootstrap", {});
    expect(store.notice?.text).toContain("准备安装更新");
  });
  it("preserves actor scope and draft models during a healthy refresh but stops new paid actions", async () => {
    const store = useOrbitStore();
    bridge.call.mockResolvedValue([imageModel()]);
    store.acceptState(bootstrap());
    await Promise.resolve();
    const before = store.scope;
    const rotating = bootstrap("synthetic-account-a", 2);
    rotating.session.status = "refreshing";
    store.acceptState(rotating);
    expect(store.scope).toBe(before);
    expect(store.account?.id).toBe("synthetic-account-a");
    expect(store.models).toHaveLength(1);
    expect(store.ready).toBe(false);
    store.acceptState(bootstrap("synthetic-account-a", 3));
    expect(store.scope).toBe(before);
    expect(store.ready).toBe(true);
    const expired = bootstrap("synthetic-account-a", 4);
    expired.session.status = "expired";
    store.acceptState(expired);
    expect(store.scope).toBe("none");
    expect(store.models).toHaveLength(0);
  });
  it("keeps browser authorization pending through local refresh and rejects stale terminal events", () => {
    const store = useOrbitStore();
    const pending = bootstrap();
    pending.pending_authorization = {
      request_id: "current-request",
      expires_at: "2026-10-13T03:10:00Z",
    };
    store.acceptState(pending);
    expect(store.loginPending).toBe(true);
    store.acceptAuthorization({
      request_id: "former-request",
      state: "denied",
      error: null,
    });
    expect(store.loginPending).toBe(true);
    expect(store.notice).toBeNull();
    // Native state may arrive before its terminal event; retain the request ID for correlation.
    store.acceptState(bootstrap("synthetic-account-a", 2));
    store.acceptAuthorization({
      request_id: "current-request",
      state: "denied",
      error: null,
    });
    expect(store.loginPending).toBe(false);
    expect(store.notice?.text).toContain("授权已拒绝");
    expect(store.account?.id).toBe("synthetic-account-a");
  });
  it("manual refresh queries account data with a frozen revision and never invokes a paid request", async () => {
    const store = useOrbitStore();
    const state = bootstrap();
    bridge.call.mockImplementation((command) =>
      command === "list_models" ? [imageModel()] : state,
    );
    store.acceptState(state);
    await Promise.resolve();
    bridge.call.mockClear();
    await store.refresh(true);
    expect(bridge.call).toHaveBeenCalledWith("refresh_account", {
      connection_id: state.active_connection_id,
      expected_revision: 1,
    });
    expect(
      bridge.call.mock.calls.some(([name]) => name === "create_image_task"),
    ).toBe(false);
    expect(store.models).toHaveLength(1);
  });
  it("ignores delayed native errors for a former account or connection revision", () => {
    const store = useOrbitStore();
    store.acceptState(bootstrap());
    const event = {
      account_id: "synthetic-account-a",
      connection_id: "connection-synthetic-account-a",
      connection_revision: 1,
      error: {
        code: "image_input_invalid",
        message: "SYNTHETIC_SECRET_SHOULD_NOT_RENDER",
        retryable: false,
      },
    };
    store.acceptError({ ...event, account_id: "synthetic-account-b" });
    store.acceptError({ ...event, connection_revision: 0 });
    expect(store.notice).toBeNull();
    store.acceptError(event);
    expect(store.notice?.text).toContain("image_input_invalid");
    expect(store.notice?.text).not.toContain("SYNTHETIC_SECRET");
    store.clearNotice();
    store.acceptState(bootstrap("synthetic-account-b", 2));
    store.acceptError(event);
    expect(store.notice).toBeNull();
  });
  it("ignores late models from a prior account and an older whole-state response", async () => {
    const a = deferred<ModelView[]>();
    const b = deferred<ModelView[]>();
    bridge.call.mockImplementation(
      (_command, request: { connection_id: string }) =>
        request.connection_id.endsWith("account-a") ? a.promise : b.promise,
    );
    const store = useOrbitStore();
    store.acceptState(bootstrap("synthetic-account-a", 10));
    store.acceptState(bootstrap("synthetic-account-b", 11));
    b.resolve([imageModel("model-b")]);
    await Promise.resolve();
    a.resolve([imageModel("model-a")]);
    await Promise.resolve();
    expect(store.models.map((model) => model.id)).toEqual(["model-b"]);
    expect(store.acceptState(bootstrap("synthetic-account-a", 10))).toBe(false);
    expect(store.account?.id).toBe("synthetic-account-b");
  });
  it("immediately clears models after node or credential revision changes", async () => {
    const oldModels = deferred<ModelView[]>();
    const newModels = deferred<ModelView[]>();
    bridge.call
      .mockReturnValueOnce(oldModels.promise)
      .mockReturnValueOnce(newModels.promise);
    const store = useOrbitStore();
    const initial = bootstrap();
    store.acceptState(initial);
    oldModels.resolve([imageModel("old")]);
    await Promise.resolve();
    expect(store.models).toHaveLength(1);
    const switched = bootstrap("synthetic-account-a", 2);
    switched.connections[0]!.revision = 2;
    switched.connections[0]!.credential_revision = "new-credential";
    store.acceptState(switched);
    expect(store.models).toHaveLength(0);
    newModels.resolve([imageModel("new")]);
    await Promise.resolve();
    expect(store.models[0]?.id).toBe("new");
  });
  it("never renders another account’s task, history or backup even in an erroneous payload", () => {
    const store = useOrbitStore();
    const state = bootstrap("synthetic-account-b");
    state.tasks = [task()];
    state.image_history = [
      {
        id: "image",
        task_id: "synthetic-task",
        account_id: "synthetic-account-a",
        created_at: "2026-10-13",
        requested_model: "model",
        observed_model: null,
        mime: "image/png",
        width: 1,
        height: 1,
        bytes: 1,
        preview_url: null,
      },
    ];
    state.backups = [
      {
        id: "backup",
        account_id: "synthetic-account-a",
        account_label: "Fixture A",
        panel_origin: "https://panel.invalid",
        installation_id: "fixture-install",
        client: "codex",
        created_at: "",
        expires_at: "",
        file_count: 1,
        state: "available",
      },
    ];
    store.acceptState(state);
    expect(store.tasks).toEqual([]);
    expect(store.history).toEqual([]);
    expect(store.backups).toEqual([]);
    store.acceptTask(task());
    expect(store.state.tasks).toHaveLength(1);
  });
  it("does not overwrite a completed event with a late queued command acknowledgement", () => {
    const store = useOrbitStore();
    const state = bootstrap();
    state.tasks = [task("completed")];
    store.acceptState(state);
    store.acceptTask(task("queued"));
    expect(store.tasks[0]?.state).toBe("completed");
    expect(store.tasks[0]?.charge_state).toBe("pending");
  });
  it("prevents double submission while an IPC action is outstanding", async () => {
    const pending = deferred<unknown>();
    bridge.call.mockReturnValue(pending.promise);
    const store = useOrbitStore();
    const first = store.perform("paid-submit", "create_image_task", {
      connection_id: "fixture",
      expected_revision: 1,
      model_id: "synthetic-image",
      prompt: "fixture",
      parameters: {},
      input_ids: [],
    });
    expect(
      await store.perform("paid-submit", "create_image_task", {
        connection_id: "fixture",
        expected_revision: 1,
        model_id: "synthetic-image",
        prompt: "fixture",
        parameters: {},
        input_ids: [],
      }),
    ).toBeUndefined();
    expect(bridge.call).toHaveBeenCalledTimes(1);
    pending.resolve(task());
    await first;
    expect(store.busy.has("paid-submit")).toBe(false);
  });
});
