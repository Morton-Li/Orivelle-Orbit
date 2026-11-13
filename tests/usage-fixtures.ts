import type {
  UsageMetrics,
  UsagePage,
  UsageQuery,
  UsageRecord,
  UsageSummary,
} from "../src/shared/usage";
import { dateRange } from "../src/shared/usage";
export function usageMetrics(input = 12400, output = 3600): UsageMetrics {
  const quantity = (value: number) => ({ value, status: "known" as const });
  return {
    tokens: {
      state: "complete",
      total: quantity(input + output),
      input: quantity(input),
      cached_input: quantity(Math.round(input * 0.58)),
      cache_write: quantity(0),
      output: quantity(output),
      reasoning: quantity(Math.round(output * 0.3)),
    },
    image_count: null,
    reconciliation_pending_count: 0,
  };
}
export function usageSummary(query = dateRange(30)): UsageSummary {
  const start = Date.parse(query.date_from + "T00:00:00Z"),
    end = Date.parse(query.date_to + "T00:00:00Z");
  const days = Array.from(
    { length: Math.round((end - start) / 86400000) + 1 },
    (_, i) => {
      const requests = 19 + ((i * 17) % 59) + (i > 18 ? 22 : 0),
        failed = i % 7 === 0 ? 2 : 0;
      return {
        date: new Date(start + i * 86400000).toISOString().slice(0, 10),
        requests,
        succeeded: requests - failed,
        failed,
        cancelled: 0,
        rejected: 0,
        interrupted: 0,
        charges: [
          {
            currency: "USD",
            amount: i === 29 ? null : (requests * 0.00187).toFixed(8),
            known_amount: (requests * 0.00187).toFixed(8),
            pending_count: i === 29 ? 2 : 0,
          },
        ],
      };
    },
  );
  const requests = days.reduce((n, d) => n + d.requests, 0),
    failed = days.reduce((n, d) => n + d.failed, 0);
  const known = days
      .reduce((n, d) => n + Number(d.charges[0]!.known_amount), 0)
      .toFixed(8),
    pending = days.reduce((n, d) => n + d.charges[0]!.pending_count, 0);
  return {
    range: {
      ...query,
      timezone: "Asia/Shanghai",
      max_days: 90,
      defaulted: false,
    },
    summary: {
      requests,
      succeeded: requests - failed,
      failed,
      cancelled: 0,
      rejected: 0,
      interrupted: 0,
      usage: usageMetrics(requests * 3821, requests * 917),
      charges: [
        {
          currency: "USD",
          amount: pending ? null : known,
          known_amount: known,
          pending_count: pending,
        },
      ],
    },
    days,
    has_legacy_semantics: false,
    updated_at: query.date_to + "T06:18:00Z",
    generated_at: query.date_to + "T06:20:00Z",
  };
}
export function usagePage(query?: Partial<UsageQuery>): UsagePage {
  const range = {
    ...dateRange(30),
    ...query,
    timezone: query?.timezone ?? "Asia/Shanghai",
    max_days: 90,
    defaulted: false,
  };
  const second = !!query?.cursor;
  const data: UsageRecord[] = Array.from(
    { length: second ? 3 : 8 },
    (_, i): UsageRecord => {
      const pending = i === 2,
        failed = i === 5,
        image = i === 3;
      return {
        id: `demo-record-${second ? 2 : 1}-${i}`,
        request_id: `a1000000-0000-4000-8000-${String(i + (second ? 100 : 0)).padStart(12, "0")}`,
        result_state: "confirmed",
        outcome: failed ? "failed" : "succeeded",
        result_label: failed ? "失败" : "成功",
        result_note: null,
        recorded_at: `${range.date_to}T${String(6 - Math.floor(i / 3)).padStart(2, "0")}:${String(24 - i * 2).padStart(2, "0")}:00Z`,
        provider_model: {
          display_name: [
            "GPT-6.1 Sol",
            "Claude Sonnet 5",
            "GPT-6 Astra",
            "GPT Image 2.5 Flare",
          ][i % 4]!,
        },
        api_key: {
          name: i % 2 ? "Orbit · Claude" : "Orbit · Codex",
          prefix: "sk-demo…",
        },
        usage: image
          ? {
              ...usageMetrics(420, 8560),
              image_count: { value: 1, status: "known" },
            }
          : usageMetrics(8420 + i * 2104, 1837 + i * 234),
        first_byte_duration_ms: 430,
        total_duration_ms: failed ? 1260 : image ? 28400 : 2100 + i * 1200,
        charge_state: pending ? "pending" : failed ? "unbilled" : "charged",
        charges:
          pending || failed
            ? []
            : [
                {
                  amount: image
                    ? "0.04832000"
                    : (0.01823 + i * 0.00314).toFixed(8),
                  currency: "USD",
                  pending_count: 0,
                },
              ],
      };
    },
  ).filter((record) => !query?.outcome || record.outcome === query.outcome);
  return {
    range,
    data,
    prev_cursor: second ? "demo-prev" : null,
    next_cursor: second ? null : "demo-next",
    cursor_reset: false,
  };
}
