import type {
  ClientEvidence,
  ClientView,
  ConnectionView,
  Money,
  NodeView,
} from "./contracts";
import { locale, t } from "./i18n";

export function configPolicyMessage(code: string): string | null {
  if (code === "account_already_connected") return t("login.alreadyConnected");
  if (code === "original_configuration_required")
    return t("access.originalRequired");
  if (code === "configuration_conflict")
    return t("access.configurationConflict");
  if (code === "claude_mapping_unavailable")
    return t("access.claudeMappingUnavailable");
  if (code === "claude_native_migration_required")
    return t("access.claudeNativeMigrationRequired");
  if (code === "claude_signature_verification_timeout")
    return t("access.claudeVerificationTimeout");
  if (code === "managed_policy_unverified") return t("access.policyUnverified");
  if (code === "managed_policy_changed") return t("access.policyChanged");
  if (
    [
      "schema_unverified",
      "claude_library_schema_unverified",
      "claude_configuration_target_unverified",
    ].includes(code)
  )
    return t("access.claudeConfigurationUnavailable");
  if (code === "claude_configuration_target_changed")
    return t("access.claudeConfigurationChanged");
  if (code === "claude_configuration_context_unverified")
    return t("access.claudeRestoreContextUnavailable");
  if (code === "claude_configuration_context_changed")
    return t("access.claudeContextChanged");
  if (code === "claude_configuration_source_unsupported")
    return t("access.claudeSourceUnsupported");
  if (code === "codex_configuration_target_unverified")
    return t("access.codexConfigurationUnavailable");
  if (code === "codex_configuration_target_changed")
    return t("access.codexConfigurationChanged");
  if (code === "codex_binding_unavailable")
    return t("access.codexBindingUnavailable");
  if (code === "codex_reasoning_unavailable")
    return t("access.codexReasoningUnavailable");
  if (
    code === "codex_catalog_scope_invalid" ||
    code === "codex_catalog_scope_changed"
  )
    return t("access.codexCatalogChanged");
  if (code === "codex_runtime_unverified")
    return t("access.codexRuntimeUnavailable");
  if (code === "codex_runtime_changed") return t("access.codexRuntimeChanged");
  if (code === "codex_catalog_context_unverified")
    return t("access.codexRestoreContextUnavailable");
  if (code === "codex_config_override_unverified")
    return t("access.codexOverrideUnavailable");
  if (
    [
      "codex_catalog_version_unverified",
      "codex_catalog_source_unverified",
      "codex_catalog_invalid_plan",
      "codex_catalog_limit_exceeded",
      "codex_catalog_alias_unauthorized",
      "codex_catalog_dependency_invalid",
      "codex_catalog_reasoning_invalid",
      "codex_catalog_default_invalid",
      "codex_catalog_template_unavailable",
    ].includes(code)
  )
    return t("access.codexCatalogUnavailable");
  return null;
}

export function nodeName(node: NodeView, index: number): string {
  const label = node.label.trim();
  return !label ||
    /https?:\/\/|\b(?:[\w-]+\.)+[a-z]{2,}\b|\b\d{1,3}(?:\.\d{1,3}){3}\b/i.test(
      label,
    )
    ? `${t("access.node")} ${index + 1}`
    : label;
}

export function redactServiceUrls(value: string): string {
  return value.replace(/https?:\/\/[^\s"'<>,)]+/gi, t("common.serviceAddress"));
}

export function formatUpdatedAt(value: string | null): string {
  if (!value || value.length > 64 || !Number.isFinite(Date.parse(value)))
    return t("common.unknown");
  return new Intl.DateTimeFormat(locale.value, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "short",
  }).format(new Date(value));
}

export function formatDecimal(
  amount: string,
  fractionDigits?: number,
): string | null {
  if (amount.length > 96 || !/^-?\d+(?:\.\d+)?$/.test(amount)) return null;
  const negative = amount.startsWith("-");
  let [rawWhole = "0", rawFraction = ""] = (
    negative ? amount.slice(1) : amount
  ).split(".");
  if (fractionDigits !== undefined) {
    const units = BigInt(
      rawWhole +
        rawFraction.slice(0, fractionDigits).padEnd(fractionDigits, "0"),
    );
    const rounded = (
      units + (Number(rawFraction[fractionDigits]) >= 5 ? 1n : 0n)
    )
      .toString()
      .padStart(fractionDigits + 1, "0");
    rawWhole = rounded.slice(0, rounded.length - fractionDigits);
    rawFraction = rounded.slice(rawWhole.length);
  }
  const whole = rawWhole
    .replace(/^0+(?=\d)/, "")
    .replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const fraction =
    fractionDigits === undefined
      ? rawFraction.replace(/0+$/, "").padEnd(2, "0")
      : rawFraction;
  return `${negative ? "-" : ""}${whole}.${fraction}`;
}
export function formatMoney(
  value: Money | null | undefined,
  fractionDigits?: number,
): string {
  if (
    !value ||
    value.status === "unknown" ||
    (value.amount === null && value.status === "known")
  )
    return t("common.unknown");
  if (value.status === "unlimited") return t("common.unlimited");
  if (value.status === "pending") return t("common.pending");
  if (value.status === "conflict") return t("common.conflict");
  const amount =
    value.amount === null ? null : formatDecimal(value.amount, fractionDigits);
  if (amount === null) return t("common.unknown");
  return `${value.currency ?? t("common.unknown")} ${amount}`;
}
export function formatBytes(bytes: number): string {
  if (!Number.isSafeInteger(bytes) || bytes < 0) return t("common.unknown");
  if (bytes >= 1024 ** 3)
    return `${Math.round((bytes / 1024 ** 3) * 10) / 10} GiB`;
  if (bytes >= 1024 ** 2)
    return `${Math.round((bytes / 1024 ** 2) * 10) / 10} MiB`;
  return `${Math.round(bytes / 1024)} KiB`;
}
export function evidenceMatches(
  evidence: ClientEvidence | null,
  client: ClientView,
  connection: ConnectionView | null,
): boolean {
  return (
    !!evidence &&
    !!connection &&
    evidence.configuration_revision === client.configuration_revision &&
    evidence.client_version === client.version &&
    evidence.account_id === connection.account_id &&
    evidence.connection_id === connection.id &&
    evidence.connection_revision === connection.revision &&
    evidence.node_id === connection.node_id &&
    Number.isFinite(Date.parse(evidence.observed_at))
  );
}
export function clientStatus(
  client: ClientView,
  connection: ConnectionView | null,
  detected: boolean,
): { text: string; tone: "neutral" | "success" | "warning" | "error" } {
  if (!detected) return { text: t("client.pending"), tone: "neutral" };
  if (client.configuration_state === "conflict")
    return { text: t("client.conflict"), tone: "error" };
  if (client.managed === "yes")
    return { text: t("client.managed"), tone: "warning" };
  if (!client.installed)
    return { text: t("client.notInstalled"), tone: "neutral" };
  if (client.configuration_state === "unsupported")
    return { text: t("client.unsupported"), tone: "warning" };
  if (
    client.connection_state === "passed" &&
    evidenceMatches(client.connection_evidence, client, connection)
  )
    return { text: t("client.verified"), tone: "success" };
  if (
    client.loaded_state === "confirmed" &&
    evidenceMatches(client.loaded_evidence, client, connection)
  )
    return { text: t("client.loaded"), tone: "success" };
  if (client.configuration_state === "saved")
    return { text: t("client.restart"), tone: "warning" };
  return { text: t("client.notConfigured"), tone: "neutral" };
}
export function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
export function safePreviewUrl(value: string | null): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    return !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      ((url.protocol === "asset:" && url.hostname === "localhost") ||
        url.origin === "http://asset.localhost" ||
        url.origin === "https://asset.localhost")
      ? value
      : null;
  } catch {
    return null;
  }
}
