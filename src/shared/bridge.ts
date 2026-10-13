import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BootstrapView, ImageInputsEvent, IpcCommands } from "./contracts";
import { t } from "./i18n";

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
export async function call<K extends keyof IpcCommands>(
  command: K,
  request: IpcCommands[K]["args"],
): Promise<IpcCommands[K]["result"]> {
  if (!isTauri())
    throw new OrbitError("desktop_required", t("common.desktopRequired"));
  try {
    return await invoke<IpcCommands[K]["result"]>(command, { request });
  } catch (error) {
    const value =
      typeof error === "object" && error !== null
        ? (error as Record<string, unknown>)
        : null;
    const code =
      typeof value?.code === "string" && /^[a-z0-9_]{1,80}$/.test(value.code)
        ? value.code
        : "operation_failed";
    // Never display arbitrary serialized backend errors: they may contain headers or secret material.
    throw new OrbitError(code, t("common.failed"), value?.retryable === true);
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
