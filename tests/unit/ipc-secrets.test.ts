import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { call, nativeAvailable, OrbitError } from "../../src/shared/bridge";

beforeEach(() => {
  vi.stubGlobal("isTauri", true);
});
afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
});
describe("domain IPC boundary", () => {
  it("uses a typed request envelope and never exposes native error bodies", async () => {
    const calls: unknown[] = [];
    mockIPC((command, payload) => {
      calls.push({ command, payload });
      throw {
        code: "vault_denied",
        message: "SYNTHETIC_SECRET_SHOULD_NOT_RENDER",
        authorization: "SYNTHETIC_HEADER",
        retryable: false,
      };
    });
    await expect(call("bootstrap", {})).rejects.toMatchObject({
      code: "vault_denied",
      message: "操作未完成，请查看连接状态或诊断报告。",
      retryable: false,
    });
    expect(calls).toEqual([{ command: "bootstrap", payload: { request: {} } }]);
  });
  it("fails before invoking any native function in a plain browser", async () => {
    vi.stubGlobal("isTauri", false);
    const invoked = vi.fn();
    mockIPC(invoked);
    expect(nativeAvailable()).toBe(false);
    await expect(call("bootstrap", {})).rejects.toBeInstanceOf(OrbitError);
    expect(invoked).not.toHaveBeenCalled();
  });
  it("does not convert malformed backend messages into UI content", async () => {
    mockIPC(() => {
      throw "token=SYNTHETIC_SECRET";
    });
    await expect(call("bootstrap", {})).rejects.toMatchObject({
      code: "operation_failed",
      message: "操作未完成，请查看连接状态或诊断报告。",
    });
  });
});
