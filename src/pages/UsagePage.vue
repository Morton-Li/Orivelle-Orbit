<script setup lang="ts">
import { computed, ref } from "vue";
import { useOrbitStore } from "../stores/orbit";
import { useUsage } from "../composables/useUsage";
import { locale, t, type MessageKey } from "../shared/i18n";
import { formatMoney, formatUpdatedAt } from "../shared/format";
import {
  dateRange,
  type UsageMoney,
  type UsageQuantity,
  type UsageRecord,
} from "../shared/usage";
import OrbitIcon from "../components/OrbitIcon.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import StatusBadge from "../components/StatusBadge.vue";
const store = useOrbitStore();
const usage = useUsage();
const {
  draft,
  range,
  outcome,
  summary,
  records,
  summaryLoading,
  recordsLoading,
  summaryError,
  recordsError,
  rangeError,
  available,
} = usage;
const mode = ref<"requests" | "spend">("requests");
const currency = ref("");
const hovered = ref<number | null>(null);
const expanded = ref<string | null>(null);
const number = (value: number | undefined) =>
  value === undefined ? "—" : value.toLocaleString(locale.value);
const total = computed(() => summary.value?.summary);
const pending = computed(() =>
  total.value?.charges.reduce(
    (count, charge) => count + charge.pending_count,
    0,
  ),
);
function money(charge: UsageMoney, known = false) {
  return formatMoney(
    {
      amount: known ? (charge.known_amount ?? charge.amount) : charge.amount,
      currency: charge.currency,
      status: "known",
    },
    4,
  );
}
function quantity(value: UsageQuantity | null | undefined) {
  return value?.value == null ? "—" : number(value.value);
}
const currencies = computed(() =>
  (total.value?.charges ?? []).map((charge) => ({
    value: charge.currency ?? "",
    label: charge.currency ?? t("usage.unknown"),
  })),
);
const chartCurrency = computed({
  get: () =>
    currencies.value.some((item) => item.value === currency.value)
      ? currency.value
      : (currencies.value[0]?.value ?? ""),
  set: (value: string) => {
    currency.value = value;
  },
});
const points = computed(() =>
  (summary.value?.days ?? []).map((day) => {
    const charge = day.charges.find(
      (c) => (c.currency ?? "") === chartCurrency.value,
    );
    const value =
      mode.value === "requests"
        ? day.requests
        : Number(charge?.known_amount ?? charge?.amount ?? 0);
    return {
      date: day.date,
      value: Number.isFinite(value) ? value : 0,
      label:
        mode.value === "requests"
          ? `${number(day.requests)} ${t("usage.requests")}`
          : charge
            ? money(charge, true)
            : `${chartCurrency.value} 0`,
      pending: charge?.pending_count ?? 0,
    };
  }),
);
const maximum = computed(
  () => Math.max(0, ...points.value.map((point) => point.value)) || 1,
);
const chartEmpty = computed(() =>
  points.value.every((point) => point.value === 0),
);
const labels = computed(() =>
  !points.value.length
    ? []
    : [
        ...new Set([
          0,
          Math.floor((points.value.length - 1) / 2),
          points.value.length - 1,
        ]),
      ].filter((i) => i >= 0),
);
function axis(value: number) {
  return value >= 1000000
    ? `${(value / 1000000).toFixed(1)}m`
    : value >= 1000
      ? `${(value / 1000).toFixed(1)}k`
      : value.toLocaleString(locale.value, { maximumFractionDigits: 3 });
}
function presetActive(days: number) {
  const next = dateRange(days);
  return (
    range.value.date_from === next.date_from &&
    range.value.date_to === next.date_to
  );
}
const selectedOutcome = computed({
  get: () => outcome.value || "all",
  set: (value: string) => {
    outcome.value = value === "all" ? "" : value;
  },
});
const outcomes = computed(() => [
  { value: "all", label: t("usage.allOutcomes") },
  ...(
    ["succeeded", "failed", "cancelled", "rejected", "interrupted"] as const
  ).map((value) => ({
    value,
    label: t(value === "succeeded" ? "usage.success" : `usage.${value}`),
  })),
]);
function resultLabel(record: UsageRecord) {
  const keys: Record<string, MessageKey> = {
    succeeded: "usage.success",
    failed: "usage.failed",
    cancelled: "usage.cancelled",
    rejected: "usage.rejected",
    interrupted: "usage.interrupted",
  };
  return record.result_state !== "confirmed"
    ? record.result_label
    : t(keys[record.outcome ?? ""] ?? "usage.unconfirmed");
}
function chargeLabel(record: UsageRecord) {
  const keys: Record<string, MessageKey> = {
    pending: "images.pendingCharge",
    reconciliation_pending: "images.reconciliation",
    unbilled: "images.unbilled",
    incomplete: "images.incomplete",
    unavailable: "usage.unknown",
    recorded: "usage.recorded",
  };
  return t(keys[record.charge_state] ?? "usage.unknown");
}
function time(value: string) {
  return new Date(value).toLocaleString(locale.value, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}
function duration(value: number | null) {
  return value === null
    ? "—"
    : `${(value / 1000).toLocaleString(locale.value, { maximumFractionDigits: 1 })}s`;
}
</script>
<template>
  <section class="page-heading usage-heading">
    <div>
      <p class="usage-eyebrow">{{ t("usage.scope") }}</p>
      <h1>{{ t("usage.title") }}</h1>
      <p class="page-description">{{ t("usage.description") }}</p>
    </div>
    <button
      class="button secondary"
      :disabled="!available || summaryLoading || recordsLoading"
      @click="usage.refresh"
    >
      <OrbitIcon name="refresh" :size="15" />{{ t("usage.refresh") }}
    </button>
  </section>
  <section
    v-if="!store.account || !store.activeConnection"
    class="usage-empty usage-login"
  >
    <span class="usage-empty-symbol"
      ><OrbitIcon name="chart" :size="28"
    /></span>
    <h2>{{ t("usage.login") }}</h2>
    <p>{{ t("usage.loginHint") }}</p>
    <button
      v-if="store.canLogin"
      class="button primary"
      @click="store.loginOpen = true"
    >
      {{ t("action.login") }}
    </button>
  </section>
  <template v-else>
    <form class="usage-filter" @submit.prevent="usage.applyRange">
      <div class="usage-segment" :aria-label="t('usage.rangeLabel')">
        <button
          v-for="(days, index) in [1, 7, 30]"
          :key="days"
          type="button"
          :class="{ selected: presetActive(days) }"
          :aria-pressed="presetActive(days)"
          @click="usage.preset(days)"
        >
          {{
            t((["usage.today", "usage.week", "usage.month"] as const)[index]!)
          }}
        </button>
      </div>
      <div class="usage-date-fields">
        <input
          id="usage-from"
          v-model="draft.date_from"
          type="date"
          :aria-label="t('usage.from')"
        /><span aria-hidden="true">—</span
        ><input
          id="usage-to"
          v-model="draft.date_to"
          type="date"
          :aria-label="t('usage.to')"
        /><button class="button ghost" type="submit">
          {{ t("usage.apply") }}
        </button>
      </div>
    </form>
    <p v-if="rangeError" class="error-text" role="alert">{{ rangeError }}</p>
    <p v-if="summaryError" class="usage-error" role="alert">
      {{ summaryError }}
      <button class="text-button" @click="usage.refresh">
        {{ t("action.refresh") }}
      </button>
    </p>
    <section
      class="usage-stat-grid"
      :aria-busy="summaryLoading"
      :class="{ 'usage-updating': summaryLoading }"
    >
      <article class="usage-stat">
        <div class="usage-stat-label">
          {{ t("usage.requests") }}<OrbitIcon name="chart" :size="16" />
        </div>
        <strong>{{ number(total?.requests) }}</strong>
        <p>{{ t("usage.requestHint") }}</p>
      </article>
      <article class="usage-stat">
        <div class="usage-stat-label">
          {{ t("usage.tokens") }}<OrbitIcon name="spark" :size="16" />
        </div>
        <strong
          >{{ quantity(total?.usage.tokens.total)
          }}<span
            v-if="total?.usage.tokens.total.status === 'partial'"
            class="usage-partial"
            >{{ t("usage.partial") }}</span
          ></strong
        >
        <p>{{ t("usage.tokenHint") }}</p>
      </article>
      <article class="usage-stat usage-stat-accent">
        <div class="usage-stat-label">
          {{ t("usage.spend") }}<OrbitIcon name="file" :size="16" />
        </div>
        <strong
          v-for="(charge, i) in total?.charges"
          :key="i"
          class="usage-money"
          >{{ money(charge, true) }}</strong
        ><strong v-if="!total?.charges.length">—</strong>
        <p>{{ t("usage.spendHint") }}</p>
      </article>
      <article class="usage-stat">
        <div class="usage-stat-label">
          {{ t("usage.pending") }}<OrbitIcon name="clock" :size="16" />
        </div>
        <strong
          >{{ number(pending)
          }}<span v-if="pending && pending > 0" class="usage-pending-dot"></span
        ></strong>
        <p>{{ t("usage.pendingHint") }}</p>
      </article>
    </section>
    <section class="usage-panel usage-trend" :aria-busy="summaryLoading">
      <div class="usage-panel-heading">
        <div>
          <h2>{{ t("usage.trend") }}</h2>
          <p>
            {{ summary?.range.date_from ?? range.date_from }} —
            {{ summary?.range.date_to ?? range.date_to }}
          </p>
        </div>
        <div class="usage-chart-controls">
          <select
            v-if="mode === 'spend' && currencies.length > 1"
            v-model="chartCurrency"
            :aria-label="t('usage.currency')"
          >
            <option
              v-for="item in currencies"
              :key="item.value"
              :value="item.value"
            >
              {{ item.label }}
            </option>
          </select>
          <div class="usage-segment">
            <button
              type="button"
              :class="{ selected: mode === 'requests' }"
              :aria-pressed="mode === 'requests'"
              @click="mode = 'requests'"
            >
              {{ t("usage.requestMode") }}</button
            ><button
              type="button"
              :class="{ selected: mode === 'spend' }"
              :aria-pressed="mode === 'spend'"
              @click="mode = 'spend'"
            >
              {{ t("usage.spendMode") }}
            </button>
          </div>
        </div>
      </div>
      <div class="usage-chart" @mouseleave="hovered = null">
        <div class="usage-y-axis">
          <span>{{ axis(maximum) }}</span
          ><span>{{ axis(maximum / 2) }}</span
          ><span>0</span>
        </div>
        <div class="usage-chart-plot">
          <div
            v-if="hovered !== null && points[hovered]"
            class="usage-chart-tooltip"
            :style="{
              left: `${Math.max(15, Math.min(85, ((hovered + 0.5) / points.length) * 100))}%`,
            }"
          >
            <span>{{ points[hovered]!.date }}</span
            ><strong>{{ points[hovered]!.label }}</strong
            ><small v-if="points[hovered]!.pending"
              >{{ t("usage.pending") }} · {{ points[hovered]!.pending }}</small
            >
          </div>
          <svg
            viewBox="0 0 760 150"
            preserveAspectRatio="none"
            role="img"
            :aria-label="
              t(mode === 'requests' ? 'usage.requestMode' : 'usage.spendMode')
            "
          >
            <defs>
              <linearGradient
                id="usage-bar-gradient"
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="0%"
                  stop-color="var(--accent)"
                  stop-opacity=".86"
                />
                <stop
                  offset="100%"
                  stop-color="var(--accent)"
                  stop-opacity=".26"
                />
              </linearGradient>
            </defs>
            <path d="M0 1H760M0 75H760M0 149H760" class="usage-grid-line" />
            <g
              v-for="(point, i) in points"
              :key="point.date"
              tabindex="0"
              :aria-label="`${point.date}: ${point.label}`"
              @mouseenter="hovered = i"
              @focus="hovered = i"
              @blur="hovered = null"
            >
              <rect
                :x="(i * 760) / points.length"
                y="0"
                :width="760 / points.length"
                height="150"
                fill="transparent"
              />
              <rect
                :x="((i + 0.16) * 760) / points.length"
                :y="
                  149 -
                  Math.max(point.value ? 2 : 0, (point.value / maximum) * 136)
                "
                :width="Math.max(2, (0.68 * 760) / points.length)"
                :height="
                  Math.max(point.value ? 2 : 0, (point.value / maximum) * 136)
                "
                rx="3"
                fill="url(#usage-bar-gradient)"
                :opacity="hovered === null || hovered === i ? 1 : 0.5"
              />
              <title>{{ point.date }} · {{ point.label }}</title>
            </g>
          </svg>
          <p v-if="chartEmpty && !summaryLoading" class="usage-chart-empty">
            {{ t("usage.noData") }}
          </p>
          <div class="usage-x-axis">
            <span
              v-for="i in labels"
              :key="i"
              :style="{ left: `${((i + 0.5) / points.length) * 100}%` }"
              >{{ points[i]!.date.slice(5).replace("-", "/") }}</span
            >
          </div>
        </div>
      </div>
      <p class="usage-panel-footnote">
        <span class="usage-legend-dot"></span>{{ t("usage.aggregationHint") }}
      </p>
    </section>
    <section class="usage-panel usage-history" :aria-busy="recordsLoading">
      <div class="usage-panel-heading">
        <div>
          <h2>{{ t("usage.records") }}</h2>
          <p>{{ t("usage.recordsHint") }}</p>
        </div>
        <div class="usage-outcome">
          <OrbitSelect
            id="usage-outcome"
            v-model="selectedOutcome"
            :options="outcomes"
            :aria-label="t('usage.allOutcomes')"
            :disabled="recordsLoading"
          />
        </div>
      </div>
      <p v-if="recordsError" class="usage-error" role="alert">
        {{ recordsError }}
        <button class="text-button" @click="usage.loadRecords()">
          {{ t("action.refresh") }}
        </button>
      </p>
      <p
        v-if="records?.cursor_reset"
        class="field-hint usage-table-note"
        role="status"
      >
        {{ t("usage.cursorReset") }}
      </p>
      <div
        class="usage-table-scroll"
        tabindex="0"
        :aria-label="t('usage.records')"
      >
        <table class="usage-table">
          <thead>
            <tr>
              <th>{{ t("usage.time") }}</th>
              <th>{{ t("usage.model") }}</th>
              <th>{{ t("usage.result") }}</th>
              <th>{{ t("usage.quantity") }}</th>
              <th>{{ t("usage.charge") }}</th>
              <th>{{ t("usage.duration") }}</th>
              <th>
                <span class="sr-only">{{ t("usage.details") }}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <template v-for="record in records?.data" :key="record.id"
              ><tr>
                <td class="usage-time">
                  {{ time(record.recorded_at)
                  }}<small>{{ record.api_key?.name ?? "—" }}</small>
                </td>
                <td class="usage-model">
                  {{ record.provider_model.display_name }}
                </td>
                <td>
                  <StatusBadge
                    :text="resultLabel(record)"
                    :tone="
                      record.outcome === 'succeeded' &&
                      record.result_state === 'confirmed'
                        ? 'success'
                        : record.outcome === 'failed'
                          ? 'warning'
                          : 'neutral'
                    "
                  />
                </td>
                <td class="usage-quantity">
                  {{ quantity(record.usage.tokens.total)
                  }}<small v-if="record.usage.image_count?.value != null"
                    >{{ number(record.usage.image_count.value) }}
                    {{ t("usage.images") }}</small
                  ><small
                    v-else-if="record.usage.tokens.total.status === 'partial'"
                    >{{ t("usage.partial") }}</small
                  >
                </td>
                <td class="usage-charge">
                  <template
                    v-if="
                      record.charges.some((charge) => charge.amount !== null)
                    "
                    ><span v-for="(charge, i) in record.charges" :key="i">{{
                      charge.amount === null
                        ? chargeLabel(record)
                        : money(charge)
                    }}</span></template
                  ><span v-else class="muted">{{ chargeLabel(record) }}</span>
                </td>
                <td class="usage-duration">
                  {{ duration(record.total_duration_ms) }}
                </td>
                <td>
                  <button
                    class="icon-button"
                    :aria-label="
                      t(
                        expanded === record.id
                          ? 'usage.hideDetails'
                          : 'usage.details',
                      )
                    "
                    :aria-expanded="expanded === record.id"
                    @click="
                      expanded = expanded === record.id ? null : record.id
                    "
                  >
                    <OrbitIcon
                      :name="
                        expanded === record.id ? 'chevron-down' : 'chevron'
                      "
                      :size="14"
                    />
                  </button>
                </td>
              </tr>
              <tr v-if="expanded === record.id" class="usage-detail-row">
                <td colspan="7">
                  <dl class="usage-detail-grid">
                    <div class="usage-request-id">
                      <dt>{{ t("usage.requestId") }}</dt>
                      <dd>{{ record.request_id }}</dd>
                    </div>
                    <div
                      v-for="item in [
                        'input',
                        'cached_input',
                        'output',
                        'reasoning',
                      ] as const"
                      :key="item"
                    >
                      <dt>
                        {{
                          t(
                            item === "cached_input"
                              ? "usage.cached"
                              : `usage.${item}`,
                          )
                        }}
                      </dt>
                      <dd>{{ quantity(record.usage.tokens[item]) }}</dd>
                    </div>
                  </dl>
                  <p v-if="record.result_note" class="field-hint">
                    {{ record.result_note }}
                  </p>
                </td>
              </tr></template
            >
          </tbody>
        </table>
      </div>
      <div v-if="!records?.data.length" class="usage-empty">
        <OrbitIcon :name="recordsLoading ? 'loader' : 'history'" :size="25" />
        <h3>{{ recordsLoading ? t("common.loading") : t("usage.empty") }}</h3>
        <p v-if="!recordsLoading">{{ t("usage.emptyHint") }}</p>
      </div>
      <div class="usage-pagination">
        <span>{{ t("usage.filterHint") }}</span>
        <div>
          <button
            class="button ghost"
            :disabled="!records?.prev_cursor || recordsLoading"
            @click="usage.loadRecords(records!.prev_cursor!)"
          >
            {{ t("usage.previous") }}</button
          ><button
            class="button ghost"
            :disabled="!records?.next_cursor || recordsLoading"
            @click="usage.loadRecords(records!.next_cursor!)"
          >
            {{ t("usage.next") }}<OrbitIcon name="chevron" :size="12" />
          </button>
        </div>
      </div>
    </section>
    <p class="usage-footer">
      <span>{{ usage.timezone }}</span
      ><span v-if="summary?.updated_at"
        >{{ t("usage.updated") }} ·
        {{ formatUpdatedAt(summary.updated_at) }}</span
      ><span v-if="summary?.has_legacy_semantics">{{
        t("usage.legacyHint")
      }}</span>
    </p>
  </template>
</template>
