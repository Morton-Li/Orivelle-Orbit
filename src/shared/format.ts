import type {
  ClientEvidence,
  ClientView,
  ConnectionView,
  Money,
} from "./contracts";
import { locale, t } from "./i18n";

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

export function formatDecimal(amount: string): string | null {
  if (amount.length > 96 || !/^-?\d+(?:\.\d+)?$/.test(amount)) return null;
  const negative = amount.startsWith("-");
  const [rawWhole = "0", rawFraction = ""] = (
    negative ? amount.slice(1) : amount
  ).split(".");
  const whole = rawWhole
    .replace(/^0+(?=\d)/, "")
    .replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const fraction = rawFraction.replace(/0+$/, "").padEnd(2, "0");
  return `${negative ? "-" : ""}${whole}.${fraction}`;
}
export function formatMoney(value: Money | null | undefined): string {
  if (
    !value ||
    value.status === "unknown" ||
    (value.amount === null && value.status === "known")
  )
    return t("common.unknown");
  if (value.status === "unlimited") return t("common.unlimited");
  if (value.status === "pending") return t("common.pending");
  if (value.status === "conflict") return t("common.conflict");
  const amount = value.amount === null ? null : formatDecimal(value.amount);
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
export function canonicalPanelOrigin(value: string): string | null {
  try {
    const url = new URL(value.trim());
    return url.protocol === "https:" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      url.pathname === "/"
      ? url.origin
      : null;
  } catch {
    return null;
  }
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
