import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import { call } from "../shared/bridge";
import type { NodeLatencyView, NodeView } from "../shared/contracts";
import { locale, t } from "../shared/i18n";

const refreshInterval = 30_000;
const staleAfter = 60_000;

export function useNodeLatency() {
  const store = useOrbitStore();
  const measurements = ref<Record<string, NodeLatencyView>>({});
  const refreshing = ref(false);
  const now = ref(Date.now());
  const visible = ref(document.visibilityState === "visible");
  const foreground = ref(true);
  const mounted = ref(false);
  const updatedAt = ref<number | null>(null);
  const endpoints = new Map<string, string>();
  let generation = 0;
  let pending: { id: string; promise: Promise<void> } | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  const owner = computed(() => {
    const connection = store.activeConnection;
    return connection
      ? JSON.stringify([
          connection.panel_origin,
          connection.account_id,
          connection.id,
        ])
      : "";
  });
  const targets = computed(() =>
    store.state.nodes.filter((node) => node.available),
  );
  const scope = computed(() =>
    JSON.stringify([
      owner.value,
      store.scope,
      store.state.session.status,
      store.state.maintenance_active,
      targets.value.map((node) => [node.id, node.revision, node.base_url]),
    ]),
  );
  const active = computed(
    () =>
      mounted.value &&
      store.native &&
      visible.value &&
      foreground.value &&
      store.state.session.status === "authenticated" &&
      !store.state.maintenance_active &&
      !!owner.value &&
      targets.value.length > 0,
  );

  function stop() {
    generation++;
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
    const cancelled = pending;
    pending = undefined;
    refreshing.value = false;
    if (cancelled) void cancel(cancelled.id);
  }
  async function cancel(id: string) {
    try {
      await call("cancel_node_probes", { probe_id: id });
    } catch {
      // A retired run can never publish into the current scope, even if cancellation fails.
    }
  }

  function refresh(): Promise<void> {
    if (!active.value) return Promise.resolve();
    if (pending) return pending.promise;
    const connection = store.activeConnection!;
    const capturedScope = scope.value;
    const currentGeneration = ++generation;
    const nodes = [...targets.value];
    const id = crypto.randomUUID();
    refreshing.value = true;
    const current = () =>
      currentGeneration === generation &&
      capturedScope === scope.value &&
      active.value;
    const promise = (async () => {
      try {
        const results = await call("probe_nodes", {
          connection_id: connection.id,
          expected_revision: connection.revision,
          node_ids: nodes.map((node) => node.id),
          probe_id: id,
        });
        if (!current()) return;
        const received = new Map(
          results.map((result) => [result.node_id, result]),
        );
        for (const node of nodes) {
          const result = received.get(node.id);
          if (result?.node_revision === node.revision) {
            measurements.value[node.id] = result;
            endpoints.set(node.id, node.base_url);
          }
        }
        updatedAt.value = Date.now();
        now.value = Date.now();
      } catch {
        if (!current()) return;
        const measuredAt = new Date().toISOString();
        for (const node of nodes) {
          measurements.value[node.id] = {
            node_id: node.id,
            node_revision: node.revision,
            status: "failed",
            milliseconds: null,
            measured_at: measuredAt,
          };
          endpoints.set(node.id, node.base_url);
        }
        updatedAt.value = Date.now();
        now.value = Date.now();
      } finally {
        if (currentGeneration === generation) {
          pending = undefined;
          refreshing.value = false;
        }
      }
    })();
    pending = { id, promise };
    return promise;
  }

  watch(
    owner,
    () => {
      measurements.value = {};
      endpoints.clear();
      updatedAt.value = null;
    },
    { flush: "sync" },
  );
  watch(
    [scope, active],
    () => {
      stop();
      for (const [id, result] of Object.entries(measurements.value)) {
        if (
          !targets.value.some(
            (node) =>
              node.id === id &&
              node.revision === result.node_revision &&
              node.base_url === endpoints.get(id),
          )
        ) {
          delete measurements.value[id];
          endpoints.delete(id);
        }
      }
      if (
        !["authenticated", "refreshing"].includes(store.state.session.status)
      ) {
        measurements.value = {};
        updatedAt.value = null;
      }
      if (!active.value) return;
      now.value = Date.now();
      if (
        updatedAt.value === null ||
        now.value - updatedAt.value >= refreshInterval ||
        targets.value.some((node) => !measurements.value[node.id])
      )
        void refresh();
      timer = setInterval(() => {
        now.value = Date.now();
        void refresh();
      }, refreshInterval);
    },
    { flush: "sync" },
  );

  function visibilityChanged() {
    visible.value = document.visibilityState === "visible";
  }
  function focus() {
    foreground.value = true;
  }
  function blur() {
    foreground.value = false;
  }
  onMounted(() => {
    document.addEventListener("visibilitychange", visibilityChanged);
    window.addEventListener("focus", focus);
    window.addEventListener("blur", blur);
    mounted.value = true;
  });
  onUnmounted(() => {
    mounted.value = false;
    stop();
    document.removeEventListener("visibilitychange", visibilityChanged);
    window.removeEventListener("focus", focus);
    window.removeEventListener("blur", blur);
  });

  function label(node: NodeView): string {
    if (!node.available) return t("access.nodeUnavailable");
    const result = measurements.value[node.id];
    if (!result)
      return t(
        refreshing.value ? "access.latencyMeasuring" : "access.latencyUnknown",
      );
    const text =
      result.status === "available"
        ? `${result.milliseconds} ms`
        : t(
            result.status === "timeout"
              ? "access.latencyTimeout"
              : "access.latencyFailed",
          );
    return now.value - Date.parse(result.measured_at) >= staleAfter
      ? `${text} · ${t("access.latencyStale")}`
      : text;
  }
  const updatedLabel = computed(() =>
    updatedAt.value === null
      ? ""
      : `${t("access.latencyUpdated")} ${new Date(updatedAt.value).toLocaleTimeString(locale.value, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}`,
  );
  return { measurements, refreshing, active, refresh, label, updatedLabel };
}
