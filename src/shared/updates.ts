import { OrbitError } from "./bridge";
import { t } from "./i18n";

export function updateFailureMessage(
  error: unknown,
  operation: "check" | "download" | "install",
): string {
  const code = error instanceof OrbitError ? error.code : "operation_failed";
  if (code === "update_download_cancelled")
    return t("settings.downloadCancelled");
  if (code === "update_busy") return t("settings.updateBlocked");
  if (code === "update_request_timeout" || code === "result_unconfirmed")
    return t(
      operation === "check"
        ? "settings.checkTimeout"
        : "settings.downloadTimeout",
    );
  if (
    [
      "invalid_update_signature",
      "update_version_signature_mismatch",
      "update_artifact_changed",
      "update_resource_limit",
      "response_too_large",
    ].includes(code)
  )
    return t("settings.invalidUpdatePackage");
  return t(
    operation === "check"
      ? "settings.updateCheckFailed"
      : operation === "download"
        ? "settings.downloadFailed"
        : "settings.updateFailed",
  );
}
