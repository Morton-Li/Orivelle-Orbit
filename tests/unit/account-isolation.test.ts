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
import type {
  AuthorizationEvent,
  LoginStart,
  ModelView,
} from "../../src/shared/contracts";
function expiredAccount(revision = 1) {
  const state = bootstrap("synthetic-account-a", revision);
  state.session.status = "expired";
  state.session.account = null;
  return state;
}
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
  it.each(
    (
      ["connected", "denied", "expired", "failed", "cancelled"] as const
    ).flatMap((terminal) =>
      [true, false].map((pendingSeen) => ({ terminal, pendingSeen })),
    ),
  )(
    "does not revive $terminal after a late login start acknowledgement; pending state seen=$pendingSeen",
    async ({ terminal, pendingSeen }) => {
      const store = useOrbitStore();
      const acknowledgement = deferred<LoginStart>();
      const finished =
        terminal === "connected"
          ? bootstrap("synthetic-account-a", 3)
          : expiredAccount(3);
      bridge.call.mockImplementation((command) => {
        if (command === "start_login") return acknowledgement.promise;
        return command === "bootstrap" ? finished : [];
      });
      store.acceptState(expiredAccount());
      store.loginOpen = true;
      const starting = store.login();
      if (pendingSeen) {
        const pending = expiredAccount(2);
        pending.pending_authorization = {
          request_id: "current-login",
          expires_at: new Date(Date.now() + 60_000).toISOString(),
        };
        store.acceptState(pending);
      }
      store.acceptState(finished);
      const event: AuthorizationEvent = {
        request_id: "current-login",
        state: terminal,
        error:
          terminal === "failed"
            ? {
                code: "authorization_failed",
                message: "Private",
                retryable: false,
              }
            : null,
      };
      store.acceptAuthorization(event);
      const handledNotice = store.notice;
      acknowledgement.resolve({
        request_id: event.request_id,
        expires_at: new Date(Date.now() + 60_000).toISOString(),
      });
      await starting;
      expect(store.state.state_revision).toBe(finished.state_revision);
      expect(store.loginPending).toBe(false);
      expect(store.loginRequestId).toBeNull();
      expect(store.loginExpiresAt).toBeNull();
      expect(store.loginOpen).toBe(terminal !== "connected");
      expect(store.notice).not.toBeNull();
      if (pendingSeen) expect(store.notice).toEqual(handledNotice);
      expect(store.notice?.text).not.toContain("Private");
      expect(
        bridge.call.mock.calls.filter(([name]) => name === "start_login"),
      ).toHaveLength(1);
    },
  );
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
  it("isolates early terminal events across login retries and ignores duplicates", async () => {
    const store = useOrbitStore();
    const first = deferred<LoginStart>();
    const second = deferred<LoginStart>();
    const pending = expiredAccount(2);
    pending.pending_authorization = {
      request_id: "first-login",
      expires_at: new Date(Date.now() + 60_000).toISOString(),
    };
    let current = pending;
    let starts = 0;
    bridge.call.mockImplementation((command) => {
      if (command === "start_login")
        return ++starts === 1 ? first.promise : second.promise;
      return command === "bootstrap" ? current : [];
    });
    store.acceptState(expiredAccount());
    store.loginOpen = true;
    const startingFirst = store.login();
    expect(await store.login()).toBeUndefined();
    store.acceptAuthorization({
      request_id: "old-login",
      state: "connected",
      error: null,
    });
    expect(store.loginOpen).toBe(true);
    first.resolve(pending.pending_authorization);
    await startingFirst;
    expect(store.loginPending).toBe(true);
    const denied: AuthorizationEvent = {
      request_id: "first-login",
      state: "denied",
      error: null,
    };
    store.acceptAuthorization(denied);
    const notice = store.notice;
    store.acceptAuthorization({ ...denied, state: "connected" });
    expect(store.notice).toEqual(notice);
    expect(store.loginOpen).toBe(true);
    current = expiredAccount(3);
    store.acceptState(current);
    const startingSecond = store.login();
    store.acceptAuthorization({
      request_id: "second-login",
      state: "connected",
      error: null,
    });
    store.acceptAuthorization(denied);
    second.resolve({
      request_id: "second-login",
      expires_at: pending.pending_authorization.expires_at,
    });
    await startingSecond;
    expect(starts).toBe(2);
    expect(store.loginPending).toBe(false);
    expect(store.loginRequestId).toBeNull();
    expect(store.loginOpen).toBe(false);
    expect(store.notice?.text).toBe("账户连接已完成。");
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
  it("recovers a first account-cache miss with an origin-bound read while paid actions stay disabled", async () => {
    const store = useOrbitStore();
    const initial = bootstrap();
    initial.session.account = null;
    const refreshed = bootstrap("synthetic-account-a", 2);
    const response = deferred<typeof refreshed>();
    bridge.call.mockImplementation((command) => {
      if (command === "refresh_account") return response.promise;
      if (command === "list_models") return [imageModel()];
      return initial;
    });
    store.acceptState(initial);
    expect(store.activeConnection).toBeNull();
    expect(store.ready).toBe(false);
    const reading = store.refresh(true);
    await vi.waitFor(() =>
      expect(bridge.call).toHaveBeenCalledWith("refresh_account", {
        connection_id: initial.active_connection_id,
        expected_revision: 1,
      }),
    );
    expect(store.ready).toBe(false);
    response.resolve(refreshed);
    await reading;
    expect(store.account?.id).toBe("synthetic-account-a");
    expect(store.ready).toBe(true);
    expect(
      bridge.call.mock.calls.some(([name]) => name === "create_image_task"),
    ).toBe(false);
  });
  it("does not recover an account read through an expired session, another origin or a mismatched cached actor", async () => {
    for (const mismatch of ["expired", "origin", "actor"]) {
      setActivePinia(createPinia());
      const store = useOrbitStore();
      const state = bootstrap();
      state.session.account = null;
      if (mismatch === "expired") state.session.status = "expired";
      if (mismatch === "origin")
        state.session.panel_origin = "https://other.invalid";
      if (mismatch === "actor")
        state.session.account = bootstrap(
          "synthetic-account-b",
        ).session.account;
      bridge.call.mockReset();
      bridge.call.mockResolvedValue(state);
      store.acceptState(state);
      await store.refresh(true);
      expect(bridge.call).toHaveBeenCalledTimes(1);
      expect(bridge.call).toHaveBeenCalledWith("bootstrap", {});
      expect(store.ready).toBe(false);
    }
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
    state.switch_snapshots = [
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
    expect(store.switchSnapshots).toEqual([]);
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
