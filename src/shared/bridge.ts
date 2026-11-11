import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  BootstrapView,
  AuthorizationEvent,
  ImageInputsEvent,
  IpcCommands,
  ScopedErrorEvent,
} from "./contracts";
import { t } from "./i18n";
import { imageFailureMessage } from "./image-parameters";

export class OrbitError extends Error {
  constructor(
    public code: string,
    message: string,
    public retryable = false,
  ) {
    super(message);
  }
}
export function nativeAvailable(): boolean {
  return isTauri();
}
export function publicFailure(error: unknown): OrbitError {
  const value =
    typeof error === "object" && error !== null
      ? (error as Record<string, unknown>)
      : null;
  const code =
    typeof value?.code === "string" && /^[a-z0-9_]{1,80}$/.test(value.code)
      ? value.code
      : "operation_failed";
  // Native and network messages can contain paths, headers or secret material.
  return new OrbitError(
    code,
    imageFailureMessage(code),
    value?.retryable === true,
  );
}
export async function call<K extends keyof IpcCommands>(
  command: K,
  request: IpcCommands[K]["args"],
): Promise<IpcCommands[K]["result"]> {
  if (!isTauri())
    throw new OrbitError("desktop_required", t("common.desktopRequired"));
  try {
    return await invoke<IpcCommands[K]["result"]>(command, { request });
  } catch (error) {
    throw publicFailure(error);
  }
}
export async function subscribeState(
  callback: (state: BootstrapView) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<BootstrapView>("orbit://state", (event) =>
    callback(event.payload),
  );
}
export async function subscribeImageInputs(
  callback: (event: ImageInputsEvent) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<ImageInputsEvent>("orbit://image-inputs", (event) =>
    callback(event.payload),
  );
}
export async function subscribeErrors(
  callback: (event: ScopedErrorEvent) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<ScopedErrorEvent>("orbit://error", (event) => {
    const payload = event.payload;
    if (!payload || typeof payload !== "object") return;
    const global =
      payload.account_id === null &&
      payload.connection_id === null &&
      payload.connection_revision === null;
    const scoped =
      typeof payload.account_id === "string" &&
      typeof payload.connection_id === "string" &&
      Number.isSafeInteger(payload.connection_revision) &&
      payload.connection_revision !== null &&
      payload.connection_revision >= 0;
    if (!global && !scoped) return;
    const failure = publicFailure(payload.error);
    callback({
      ...payload,
      error: {
        code: failure.code,
        message: failure.message,
        retryable: failure.retryable,
      },
    });
  });
}
export async function subscribeAuthorization(
  callback: (event: AuthorizationEvent) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<AuthorizationEvent>("orbit://authorization", (event) => {
    const payload = event.payload;
    if (
      !payload ||
      typeof payload.request_id !== "string" ||
      !/^[a-zA-Z0-9_-]{1,128}$/.test(payload.request_id) ||
      !["connected", "denied", "expired", "failed", "cancelled"].includes(
        payload.state,
      )
    )
      return;
    const failure =
      payload.error === null ? null : publicFailure(payload.error);
    callback({
      request_id: payload.request_id,
      state: payload.state,
      error: failure
        ? {
            code: failure.code,
            message: failure.message,
            retryable: failure.retryable,
          }
        : null,
    });
  });
}
