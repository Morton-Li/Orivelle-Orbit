import { computed, onUnmounted, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import { call, OrbitError } from "../shared/bridge";
import {
  dateRange,
  validUsageRange,
  type UsagePage,
  type UsageQuery,
  type UsageSummary,
} from "../shared/usage";
import { t } from "../shared/i18n";
export function useUsage() {
  const store = useOrbitStore();
  const range = ref(dateRange(30));
  const draft = ref({ ...range.value });
  const outcome = ref("");
  const summary = ref<UsageSummary | null>(null),
    records = ref<UsagePage | null>(null);
  const summaryLoading = ref(false),
    recordsLoading = ref(false);
  const summaryError = ref(""),
    recordsError = ref(""),
    rangeError = ref("");
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  let summaryGeneration = 0,
    recordsGeneration = 0;
  const scope = computed(() =>
    store.account && store.activeConnection
      ? `${store.account.id}/${store.activeConnection.panel_origin}/${store.activeConnection.id}/${store.activeConnection.revision}`
      : "none",
  );
  const available = computed(
    () =>
      store.native &&
      !!store.account &&
      !!store.activeConnection &&
      store.state.session.status === "authenticated",
  );
  function request(cursor?: string): UsageQuery {
    return {
      connection_id: store.activeConnection!.id,
      expected_revision: store.activeConnection!.revision,
      ...range.value,
      timezone,
      ...(outcome.value ? { outcome: outcome.value } : {}),
      ...(cursor ? { cursor } : {}),
    };
  }
  async function loadSummary() {
    if (!available.value) return;
    const generation = ++summaryGeneration,
      before = scope.value;
    summaryLoading.value = true;
    summaryError.value = "";
    try {
      const value = await call("read_usage_summary", request());
      if (generation === summaryGeneration && before === scope.value)
        summary.value = value;
    } catch (error) {
      if (generation === summaryGeneration && before === scope.value)
        summaryError.value = t(
          error instanceof OrbitError && error.code === "endpoint_unavailable"
            ? "usage.summaryUnavailable"
            : "usage.loadFailed",
        );
    } finally {
      if (generation === summaryGeneration) summaryLoading.value = false;
    }
  }
  async function loadRecords(cursor?: string) {
    if (!available.value) return;
    const generation = ++recordsGeneration,
      before = scope.value;
    recordsLoading.value = true;
    recordsError.value = "";
    try {
      const value = await call("list_usage_records", request(cursor));
      if (generation === recordsGeneration && before === scope.value)
        records.value = value;
    } catch {
      if (generation === recordsGeneration && before === scope.value)
        recordsError.value = t("usage.loadFailed");
    } finally {
      if (generation === recordsGeneration) recordsLoading.value = false;
    }
  }
  function refresh() {
    void loadSummary();
    void loadRecords();
  }
  function applyRange() {
    if (!validUsageRange(draft.value.date_from, draft.value.date_to)) {
      rangeError.value = t("usage.invalidRange");
      return;
    }
    rangeError.value = "";
    range.value = { ...draft.value };
    refresh();
  }
  function preset(days: number) {
    draft.value = dateRange(days);
    applyRange();
  }
  watch(outcome, () => void loadRecords());
  watch(
    [scope, available],
    ([next, enabled], old) => {
      if (next === old?.[0]) {
        if (enabled) refresh();
        return;
      }
      summaryGeneration++;
      recordsGeneration++;
      summary.value = null;
      records.value = null;
      summaryLoading.value = false;
      recordsLoading.value = false;
      summaryError.value = "";
      recordsError.value = "";
      if (available.value) refresh();
    },
    { immediate: true, flush: "sync" },
  );
  onUnmounted(() => {
    summaryGeneration++;
    recordsGeneration++;
  });
  return {
    range,
    draft,
    outcome,
    summary,
    records,
    summaryLoading,
    recordsLoading,
    summaryError,
    recordsError,
    rangeError,
    available,
    timezone,
    applyRange,
    preset,
    refresh,
    loadRecords,
  };
}
