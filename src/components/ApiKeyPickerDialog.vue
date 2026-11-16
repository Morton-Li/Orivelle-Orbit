<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../shared/i18n";
import { call, publicFailure } from "../shared/bridge";
import type { ApiKeyPage, ApiKeyView } from "../shared/api-keys";
import { useOrbitStore } from "../stores/orbit";
import { useApiKeysStore } from "../stores/api-keys";
import OrbitDialog from "./OrbitDialog.vue";
import StatusBadge from "./StatusBadge.vue";
const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{
  "update:open": [value: boolean];
  selected: [key: ApiKeyView];
}>();
const orbit = useOrbitStore();
const api = useApiKeysStore();
const search = ref("");
const number = ref(1);
const response = ref<ApiKeyPage | null>(null);
const selected = ref("");
const loading = ref(false);
const error = ref("");
let generation = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
const chosen = computed(() =>
  response.value?.items.find((key) => key.id === selected.value),
);
const labels = computed<Record<string, string>>(() => ({
  active: t("keys.active"),
  spending_exhausted: t("keys.exhausted"),
  expired: t("keys.expired"),
  revoked: t("keys.revoked"),
  risk_suspended: t("keys.risk"),
  retired: t("keys.retired"),
}));
async function load() {
  const connection = orbit.activeConnection;
  if (!props.open || !connection) return;
  const request = ++generation;
  const scope = orbit.scope;
  loading.value = true;
  error.value = "";
  try {
    const page = await call("list_api_keys", {
      connection_id: connection.id,
      expected_revision: connection.revision,
      search: search.value.trim(),
      status: null,
      page: number.value,
    });
    if (request !== generation || scope !== orbit.scope || !props.open) return;
    response.value = page;
  } catch (reason) {
    if (request === generation && props.open)
      error.value = publicFailure(reason).message;
  } finally {
    if (request === generation) loading.value = false;
  }
}
async function choose() {
  const key = chosen.value;
  if (
    !key ||
    !key.selectable ||
    key.legacy_issued ||
    !response.value?.can_write ||
    api.busy
  )
    return;
  if (await api.choose(key)) {
    emit("selected", key);
    emit("update:open", false);
  }
}
watch(
  () => props.open,
  (open) => {
    ++generation;
    selected.value = api.page?.selected?.id ?? "";
    search.value = "";
    number.value = 1;
    response.value = null;
    error.value = "";
    if (open) void load();
  },
  { immediate: true },
);
watch(search, () => {
  clearTimeout(timer);
  number.value = 1;
  timer = setTimeout(() => {
    void load();
  }, 250);
});
watch(
  () => api.identity,
  () => {
    ++generation;
    emit("update:open", false);
  },
);
onUnmounted(() => {
  ++generation;
  clearTimeout(timer);
});
</script>
<template>
  <OrbitDialog
    :open="open"
    :title="t('keys.pickerTitle')"
    :description="t('keys.pickerHint')"
    @update:open="emit('update:open', $event)"
  >
    <input
      v-model="search"
      type="search"
      class="key-picker-search"
      :aria-label="t('keys.pickerSearch')"
      :placeholder="t('keys.searchPlaceholder')"
    />
    <p v-if="error" class="field-hint" role="alert">{{ error }}</p>
    <p v-else-if="loading && !response" class="muted" role="status">
      {{ t("keys.loading") }}
    </p>
    <div v-else-if="response" class="key-choice-list" :aria-busy="loading">
      <label
        v-for="key in response.items"
        :key="key.id"
        class="key-choice"
        :class="{
          selected: key.id === selected,
          unavailable: !key.selectable || key.legacy_issued,
        }"
      >
        <input
          v-model="selected"
          type="radio"
          name="orbit-key-choice"
          :value="key.id"
          :disabled="!key.selectable || key.legacy_issued || api.busy"
        />
        <div>
          <strong>{{ key.name }}</strong
          ><span class="key-prefix mono">{{ key.prefix }}…</span
          ><small v-if="key.legacy_issued" class="field-hint">{{
            t("keys.legacyCleanup")
          }}</small>
        </div>
        <StatusBadge
          :tone="key.selectable && !key.legacy_issued ? 'success' : 'neutral'"
          :text="labels[key.status] ?? key.status"
        />
      </label>
      <p v-if="!response.items.length" class="muted">
        {{ t("keys.noMatchPeriod") }}
      </p>
      <div v-if="response.last_page > 1" class="key-pagination">
        <button
          class="text-button"
          :disabled="number <= 1 || loading"
          @click="
            number--;
            load();
          "
        >
          {{ t("keys.previous") }}
        </button>
        <span>{{ response.page }} / {{ response.last_page }}</span>
        <button
          class="text-button"
          :disabled="number >= response.last_page || loading"
          @click="
            number++;
            load();
          "
        >
          {{ t("keys.next") }}
        </button>
      </div>
    </div>
    <div class="dialog-actions">
      <button
        class="button secondary"
        :disabled="api.busy"
        @click="emit('update:open', false)"
      >
        {{ t("keys.cancel") }}
      </button>
      <button
        class="button primary"
        :disabled="
          !chosen?.selectable ||
          chosen.legacy_issued ||
          !response?.can_write ||
          loading ||
          api.busy
        "
        @click="choose"
      >
        {{ t("keys.use") }}
      </button>
    </div>
  </OrbitDialog>
</template>
