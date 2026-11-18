import type { ChargeState } from "./contracts";
export type UsageQuantity = {
  value: number | null;
  status: "known" | "partial" | "unknown" | "not_applicable";
};
export type UsageMetrics = {
  tokens: {
    state: string;
    total: UsageQuantity;
    input: UsageQuantity;
    cached_input: UsageQuantity;
    cache_write: UsageQuantity;
    cache_write_30m?: UsageQuantity;
    cache_write_12h?: UsageQuantity;
    output: UsageQuantity;
    reasoning: UsageQuantity;
  };
  image_count: UsageQuantity | null;
  reconciliation_pending_count: number;
};
export type UsageMoney = {
  amount: string | null;
  currency: string | null;
  known_amount?: string | null;
  pending_count: number;
  request_id?: string;
};
export type UsageRange = {
  date_from: string;
  date_to: string;
  timezone: string;
  defaulted: boolean;
  max_days: number;
};
export type UsageCounts = {
  requests: number;
  succeeded: number;
  failed: number;
  cancelled: number;
  rejected: number;
  interrupted: number;
};
export type UsageSummary = {
  range: UsageRange;
  summary: UsageCounts & { usage: UsageMetrics; charges: UsageMoney[] };
  days: Array<UsageCounts & { date: string; charges: UsageMoney[] }>;
  has_legacy_semantics: boolean;
  updated_at: string | null;
  generated_at: string;
};
export type UsageRecord = {
  id: string;
  request_id: string;
  result_state: string;
  outcome: string | null;
  result_label: string;
  result_note: string | null;
  recorded_at: string;
  provider_model: { display_name: string };
  api_key: { name: string; prefix: string } | null;
  usage: UsageMetrics;
  total_duration_ms: number | null;
  first_byte_duration_ms: number | null;
  charge_state: ChargeState;
  charges: UsageMoney[];
};
export type UsagePage = {
  data: UsageRecord[];
  prev_cursor: string | null;
  next_cursor: string | null;
  cursor_reset: boolean;
  range: UsageRange;
};
export type UsageQuery = {
  connection_id: string;
  expected_revision: number;
  date_from: string;
  date_to: string;
  timezone: string;
  api_key_id?: string;
  outcome?: string;
  cursor?: string;
};
export function localDate(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
export function dateRange(days: number, now = new Date()) {
  const from = new Date(now);
  from.setDate(from.getDate() - days + 1);
  return { date_from: localDate(from), date_to: localDate(now) };
}
export function validUsageRange(from: string, to: string): boolean {
  if (![from, to].every((d) => /^\d{4}-\d{2}-\d{2}$/.test(d))) return false;
  const first = Date.parse(`${from}T00:00:00Z`),
    last = Date.parse(`${to}T00:00:00Z`);
  return (
    Number.isFinite(first) &&
    Number.isFinite(last) &&
    new Date(first).toISOString().slice(0, 10) === from &&
    new Date(last).toISOString().slice(0, 10) === to &&
    last >= first &&
    last - first < 90 * 86400000
  );
}

/** Old Panels provide the aggregate only; absence never means a zero TTL bucket. */
export function usageTokenRows(tokens: UsageMetrics["tokens"]) {
  const rows: Array<{ key: string; quantity: UsageQuantity | undefined }> = [
    { key: "input", quantity: tokens.input },
    { key: "cached", quantity: tokens.cached_input },
  ];
  if (
    tokens.cache_write_30m !== undefined ||
    tokens.cache_write_12h !== undefined
  ) {
    rows.push(
      { key: "cacheWrite30m", quantity: tokens.cache_write_30m },
      { key: "cacheWrite12h", quantity: tokens.cache_write_12h },
    );
    if (
      tokens.cache_write_30m?.status !== "known" ||
      tokens.cache_write_12h?.status !== "known"
    ) {
      rows.push({ key: "cacheWrite", quantity: tokens.cache_write });
    }
  } else {
    rows.push({ key: "cacheWrite", quantity: tokens.cache_write });
  }
  rows.push(
    { key: "output", quantity: tokens.output },
    { key: "reasoning", quantity: tokens.reasoning },
  );
  return rows;
}
