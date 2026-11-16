<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "reka-ui";
import OrbitIcon from "../components/OrbitIcon.vue";
import OrbitDialog from "../components/OrbitDialog.vue";
import ApiKeyPickerDialog from "../components/ApiKeyPickerDialog.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import StatusBadge from "../components/StatusBadge.vue";
import { useApiKeysStore } from "../stores/api-keys";
import { useOrbitStore } from "../stores/orbit";
import { call } from "../shared/bridge";
import { formatDecimal } from "../shared/format";
import { localDate } from "../shared/usage";
import { locale, t } from "../shared/i18n";
import type {
  ApiKeyView,
  ApiKeyStatus,
  ServiceTier,
  ApiKeyChanges,
} from "../shared/api-keys";
import "../api-keys.css";

type Tier = ServiceTier;
type LimitMode = "unlimited" | "cumulative" | "rolling";
type ApiKey = {
  id: string;
  name: string;
  prefix: string;
  status: ApiKeyStatus;
  used: string;
  limit: string | null;
  mode: LimitMode;
  window: string;
  tier: Tier;
  expires: string;
  lastUsed: string;
  created: string;
  info: ApiKeyView;
};
type Action = "rotate" | "reset" | "revoke" | "delete";
const api = useApiKeysStore();
const store = useOrbitStore();
const router = useRouter();
const clientLabels = { codex: "Codex", claude: "Claude Desktop" };
const readOnly = computed(
  () => !api.page?.can_write || api.busy || !!api.page?.pending_operation,
);
const search = computed({
  get: () => api.search,
  set: (value) => {
    api.search = value;
  },
});
const filter = computed({
  get: () => api.status ?? "all",
  set: (value) => {
    api.status = value === "all" ? null : (value as ApiKeyStatus);
  },
});
function date(value: string | null) {
  if (!value) return "";
  const parsed = new Date(value);
  return Number.isFinite(parsed.getTime())
    ? new Intl.DateTimeFormat(locale.value, {
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
      }).format(parsed)
    : "—";
}
function adapt(info: ApiKeyView): ApiKey {
  return {
    id: info.id,
    name: info.name,
    prefix: info.prefix + "…",
    status: info.status,
    used: info.spending_active_used,
    limit: info.spending_limit,
    mode: info.spending_limit === null ? "unlimited" : info.spending_limit_mode,
    window: String(info.spending_window_minutes ?? 300),
    tier: info.service_tier ?? (info.force_fast ? "priority" : "default"),
    expires: info.expires_at ? localDate(new Date(info.expires_at)) : "",
    lastUsed: date(info.last_used_at) || t("keys.unused"),
    created: date(info.created_at),
    info,
  };
}
const keys = computed(() => api.page?.items.map(adapt) ?? []);
const filtered = keys;
const current = computed(() =>
  api.page?.selected ? adapt(api.page.selected) : undefined,
);
const defaultId = computed(() => api.page?.selected?.id ?? "");
const bound = computed(() => ({
  codex:
    api.page?.bindings.find((binding) => binding.client === "codex")
      ?.api_key_id ?? "",
  claude:
    api.page?.bindings.find((binding) => binding.client === "claude")
      ?.api_key_id ?? "",
}));
const statusLabels = computed<Record<ApiKeyStatus, string>>(() => ({
  active: t("keys.active"),
  spending_exhausted: t("keys.exhausted"),
  expired: t("keys.expired"),
  revoked: t("keys.revoked"),
  risk_suspended: t("keys.risk"),
  retired: t("keys.retired"),
}));
const tierOptions = computed<{ value: Tier; label: string }[]>(() => [
  { value: "follow_client", label: t("keys.followClient") },
  { value: "default", label: t("keys.normal") },
  { value: "priority", label: "Fast" },
  { value: "ultrafast", label: "Ultrafast" },
]);
const windowOptions = computed(() => [
  { value: "300", label: t("keys.hours5") },
  { value: "1440", label: t("keys.day1") },
  { value: "10080", label: t("keys.days7") },
]);
const switchOpen = ref(false);
const detailsOpen = ref(false);
const detail = ref<ApiKey | null>(null);
const editorOpen = ref(false);
const editingKey = ref<ApiKey | null>(null);
const editingId = computed(() => editingKey.value?.id ?? null);
const initialForm = () => ({
  name: "",
  expires: "",
  tier: "follow_client" as Tier,
  mode: "unlimited" as LimitMode,
  limit: "",
  window: "300",
  use: true,
});
const form = ref(initialForm());
const confirmOpen = ref(false);
const pending = ref<{ action: Action; key: ApiKey } | null>(null);
const pendingKey = computed(() => pending.value?.key);
const revealOpen = ref(false);
const revealKey = ref<ApiKey | null>(null);
const revealed = ref(false);
const revealedToken = ref("");
const revealing = ref(false);
let revealGeneration = 0;
let revealTimer: ReturnType<typeof setTimeout> | undefined;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
const actions = computed<
  Record<Action, { title: string; verb: string; description: string }>
>(() => ({
  rotate: {
    title: t("keys.rotateTitle"),
    verb: t("keys.rotateConfirm"),
    description: t("keys.rotateHint"),
  },
  reset: {
    title: t("keys.reset"),
    verb: t("keys.resetConfirm"),
    description: t("keys.resetHint"),
  },
  revoke: {
    title: t("keys.revokeTitle"),
    verb: t("keys.revokeConfirm"),
    description: t("keys.revokeHint"),
  },
  delete: {
    title: t("keys.deleteTitle"),
    verb: t("keys.deleteConfirm"),
    description: t("keys.deleteHint"),
  },
}));
const pendingAction = computed(
  () => actions.value[pending.value?.action ?? "revoke"],
);
const money = (value: string) =>
  (api.page?.currency === "USD" ? "$" : (api.page?.currency ?? "") + " ") +
  formatDecimal(value, 2);
const tierLabel = (tier: Tier) =>
  tierOptions.value.find((option) => option.value === tier)?.label;
const keyName = (id: string) =>
  keys.value.find((key) => key.id === id)?.name ??
  api.page?.bindings.find((binding) => binding.api_key_id === id)?.name ??
  t("keys.unbound");
const tone = (status: ApiKeyStatus) =>
  status === "active"
    ? "success"
    : ["revoked", "retired"].includes(status)
      ? "neutral"
      : "warning";
const percent = (key: ApiKey) =>
  key.limit ? Math.min(100, (Number(key.used) / Number(key.limit)) * 100) : 0;
const canSelect = (key: ApiKey) =>
  key.info.selectable && !key.info.legacy_issued;
const notify = (message: string) => store.notify(message, "neutral", 4500);
async function inspect(key: ApiKey) {
  const fresh = await api.details(key.info);
  if (fresh) {
    detail.value = adapt(fresh);
    detailsOpen.value = true;
  }
}
async function choose(key: ApiKey) {
  if (readOnly.value || !canSelect(key)) return;
  if (await api.choose(key.info)) {
    switchOpen.value = false;
    notify(t("keys.chosenPrefix") + key.name + t("keys.chosenSuffix"));
  }
}
function openSwitch() {
  switchOpen.value = true;
}
function edit(key?: ApiKey) {
  detailsOpen.value = false;
  editingKey.value = key ?? null;
  form.value = {
    name: key?.name ?? "",
    expires: key?.expires ?? "",
    tier: key?.tier ?? "follow_client",
    mode: key?.mode ?? "unlimited",
    limit: key?.limit ?? "",
    window: key?.window ?? "300",
    use: true,
  };
  editorOpen.value = true;
}
async function save() {
  if (readOnly.value || !form.value.name.trim()) return;
  let expires: string | null = null;
  if (form.value.expires)
    expires =
      editingKey.value?.expires === form.value.expires
        ? editingKey.value.info.expires_at
        : new Date(form.value.expires + "T23:59:59").toISOString();
  const changes: ApiKeyChanges = {
    name: form.value.name.trim(),
    expires_at: expires,
    service_tier: form.value.tier,
    spending_limit: form.value.mode === "unlimited" ? null : form.value.limit,
    spending_limit_mode:
      form.value.mode === "rolling" ? "rolling" : "cumulative",
    spending_window_minutes:
      form.value.mode === "rolling" ? Number(form.value.window) : null,
  };
  const result = await api.mutate(
    editingKey.value ? "update" : "create",
    editingKey.value?.info ?? null,
    changes,
  );
  if (!result) return;
  editorOpen.value = false;
  if (!editingKey.value && form.value.use && result.api_key) {
    if (await api.choose(result.api_key)) notify(t("keys.createdSelected"));
    else notify(t("keys.createdChoose"));
  } else notify(editingKey.value ? t("keys.settingsSaved") : t("keys.created"));
}
function requestAction(action: Action, key: ApiKey) {
  detailsOpen.value = false;
  pending.value = { action, key };
  confirmOpen.value = true;
}
async function confirm() {
  if (!pending.value || readOnly.value) return;
  const { action, key } = pending.value;
  const result = await api.mutate(action, key.info);
  if (!result) return;
  confirmOpen.value = false;
  notify(
    {
      reset: t("keys.counterReset"),
      revoke: t("keys.keyRevoked"),
      delete: t("keys.keyDeleted"),
      rotate: t("keys.keyRotated"),
    }[action],
  );
}
function hideSecret() {
  ++revealGeneration;
  revealed.value = false;
  revealedToken.value = "";
  clearTimeout(revealTimer);
}
function openReveal(key: ApiKey) {
  if (!api.page?.can_reveal || key.status === "retired") return;
  hideSecret();
  detailsOpen.value = false;
  revealKey.value = key;
  revealOpen.value = true;
}
async function reveal() {
  const key = revealKey.value;
  if (!key || revealing.value) return;
  const generation = ++revealGeneration;
  revealing.value = true;
  try {
    const connection = store.activeConnection;
    const identity = api.identity;
    if (!connection) return;
    const secret = await call("reveal_api_key", {
      connection_id: connection.id,
      expected_revision: connection.revision,
      api_key_id: key.id,
      key_revision: key.info.key_revision,
    });
    if (identity !== api.identity) return;
    if (
      !secret ||
      generation !== revealGeneration ||
      !revealOpen.value ||
      revealKey.value?.id !== secret.api_key_id
    )
      return;
    revealedToken.value = secret.plain_text_token;
    revealed.value = true;
    revealTimer = setTimeout(hideSecret, 30000);
  } catch (reason) {
    if (generation === revealGeneration) store.report(reason);
  } finally {
    revealing.value = false;
  }
}
async function copy() {
  if (revealKey.value && (await api.copy(revealKey.value.info)))
    notify(t("keys.copied"));
}
function requestUpdate(client: "codex" | "claude") {
  api.updateMapping = {
    client,
    mapping:
      api.page?.bindings.find((binding) => binding.client === client)
        ?.claude_mapping ?? null,
  };
  void router.push({
    path: "/access",
    query: { update: client },
    hash: "#client-" + client,
  });
}
async function reauthorize() {
  const connection = store.activeConnection;
  if (connection) {
    const result = await store.perform("logout", "logout", {
      connection_id: connection.id,
    });
    if (!result) return;
    await store.refresh();
  }
  store.loginOpen = true;
}
function onVisibility() {
  if (document.visibilityState !== "visible") hideSecret();
}
watch(revealOpen, (open) => {
  if (!open) {
    hideSecret();
    revealKey.value = null;
  }
});
watch(
  () => api.identity,
  () => {
    hideSecret();
    detailsOpen.value = false;
    editorOpen.value = false;
    confirmOpen.value = false;
    revealOpen.value = false;
    switchOpen.value = false;
    detail.value = null;
    form.value = initialForm();
  },
);
watch([search, filter], () => {
  clearTimeout(searchTimer);
  api.number = 1;
  searchTimer = setTimeout(() => {
    void api.load();
  }, 250);
});
onMounted(() => {
  void api.load();
  window.addEventListener("blur", hideSecret);
  document.addEventListener("visibilitychange", onVisibility);
});
onUnmounted(() => {
  hideSecret();
  clearTimeout(searchTimer);
  window.removeEventListener("blur", hideSecret);
  document.removeEventListener("visibilitychange", onVisibility);
});
</script>
<template>
  <div class="key-page">
    <section class="page-heading key-heading">
      <div>
        <h1>API Key</h1>
        <p class="page-description">{{ t("keys.description") }}</p>
      </div>
      <button class="button primary" :disabled="readOnly" @click="edit()">
        <OrbitIcon name="plus" :size="16" />{{ t("keys.create") }}
      </button>
    </section>

    <p v-if="!store.account" class="info-box">
      {{ t("keys.loginHint") }}
      <button
        class="text-button"
        :disabled="!store.native"
        @click="store.loginOpen = true"
      >
        {{ t("keys.connect") }}
      </button>
    </p>
    <div v-else-if="api.error" class="key-message" role="alert">
      <span>{{ api.error.message }}</span>
      <button
        v-if="api.error.code === 'api_key_authorization_required'"
        class="text-button"
        @click="reauthorize"
      >
        {{ t("keys.authorize") }}
      </button>
      <button
        v-else
        class="text-button"
        :disabled="api.loading"
        @click="api.load"
      >
        {{ t("keys.retry") }}
      </button>
    </div>
    <p v-if="api.loading && !api.page" class="muted" role="status">
      {{ t("keys.loading") }}
    </p>
    <div v-if="api.page?.pending_operation" class="key-message" role="status">
      <span>{{ t("keys.pending") }}</span>
      <button class="text-button" :disabled="api.busy" @click="api.recover">
        {{ t("keys.recover") }}
      </button>
    </div>
    <p
      v-if="store.account && api.page && !api.page.can_write"
      class="key-readonly"
    >
      <OrbitIcon name="shield" :size="16" />{{ t("keys.readonly") }}
    </p>

    <section
      v-if="api.page && (current || api.page.total > 0)"
      class="key-current"
      :aria-label="t('keys.currentAria')"
    >
      <div class="key-current-main">
        <div class="key-symbol"><OrbitIcon name="key" :size="23" /></div>
        <div class="key-current-info">
          <p class="key-caption">{{ t("keys.current") }}</p>
          <div class="key-current-title">
            <h2>{{ current?.name ?? t("keys.select") }}</h2>
            <StatusBadge
              v-if="current"
              :tone="tone(current.status)"
              :text="
                current.info.state === 'active' && !api.page.selected_ready
                  ? t('keys.credentialPending')
                  : statusLabels[current.status]
              "
            />
          </div>
          <p class="key-prefix mono">
            {{ current?.prefix ?? t("keys.chooseHint") }}
          </p>
        </div>
        <button
          class="button secondary key-switch"
          :disabled="readOnly"
          @click="openSwitch"
        >
          {{ t("keys.switch") }}<OrbitIcon name="chevron-down" :size="14" />
        </button>
      </div>
      <div class="key-bindings">
        <div
          v-for="client in ['codex', 'claude'] as const"
          :key="client"
          class="key-binding"
        >
          <span class="key-client-name">{{ clientLabels[client] }}</span
          ><span class="key-bound-name">{{ keyName(bound[client]) }}</span
          ><span
            v-if="
              api.page?.bindings.some(
                (binding) => binding.client === client && binding.current,
              )
            "
            class="key-binding-ok"
            ><OrbitIcon name="check" :size="13" />{{ t("keys.bound") }}</span
          ><button
            v-else
            class="text-button key-update"
            :disabled="readOnly || !api.page?.selected_ready"
            @click="requestUpdate(client)"
          >
            {{ bound[client] ? t("keys.updateAccess") : t("keys.openAccess")
            }}<OrbitIcon name="arrow" :size="13" />
          </button>
        </div>
      </div>
    </section>

    <section
      v-if="api.page"
      class="key-list"
      :aria-label="t('keys.mine')"
      :aria-busy="api.loading"
    >
      <div class="key-list-heading">
        <h2>
          {{ t("keys.mine")
          }}<span class="key-count">{{ api.page?.total ?? 0 }}</span>
        </h2>
        <button class="text-button key-refresh" @click="api.load">
          <OrbitIcon name="refresh" :size="14" />{{ t("keys.refresh") }}
        </button>
      </div>
      <div
        v-if="api.page.total > 0 || search || filter !== 'all'"
        class="key-toolbar"
      >
        <div class="key-search">
          <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="10.5" cy="10.5" r="6.5" />
            <path d="m16 16 4.5 4.5" /></svg
          ><input
            v-model="search"
            :aria-label="t('keys.search')"
            :placeholder="t('keys.searchPlaceholder')"
            type="search"
          />
        </div>
        <OrbitSelect
          id="key-status"
          v-model="filter"
          :options="[
            { value: 'all', label: t('keys.allStatuses') },
            { value: 'active', label: t('keys.active') },
            { value: 'spending_exhausted', label: t('keys.exhausted') },
            { value: 'expired', label: t('keys.expired') },
            { value: 'revoked', label: t('keys.revoked') },
            { value: 'risk_suspended', label: t('keys.risk') },
            { value: 'retired', label: t('keys.retired') },
          ]"
        />
      </div>
      <div v-if="filtered.length" class="key-table-wrap">
        <table class="key-table">
          <thead>
            <tr>
              <th>{{ t("keys.name") }}</th>
              <th>{{ t("keys.status") }}</th>
              <th>{{ t("keys.spendingLimit") }}</th>
              <th class="key-tier-column">{{ t("keys.tier") }}</th>
              <th>{{ t("keys.expiry") }}</th>
              <th>
                <span class="sr-only">{{ t("keys.actions") }}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="key in filtered"
              :key="key.id"
              :class="{ 'key-row-muted': key.status === 'revoked' }"
            >
              <td class="key-name-cell">
                <button class="key-name-button" @click="inspect(key)">
                  {{ key.name
                  }}<span
                    v-if="key.id === defaultId"
                    class="key-default-label"
                    >{{ t("keys.default") }}</span
                  ></button
                ><span class="key-prefix mono">{{ key.prefix }}</span>
                <span v-if="key.info.legacy_issued" class="key-table-subtle">{{
                  t("keys.legacy")
                }}</span>
              </td>
              <td class="key-status-cell">
                <StatusBadge
                  :tone="tone(key.status)"
                  :text="statusLabels[key.status]"
                />
              </td>
              <td class="key-spending-cell">
                <div class="key-spending">
                  <span>{{ money(key.used) }}</span
                  ><span class="key-spending-limit"
                    >/
                    {{
                      key.limit === null
                        ? t("keys.unlimited")
                        : money(key.limit)
                    }}</span
                  >
                </div>
                <div
                  v-if="key.limit"
                  class="key-progress"
                  :class="{ exhausted: key.status === 'spending_exhausted' }"
                >
                  <span :style="{ width: `${percent(key)}%` }" />
                </div>
                <span v-else class="key-table-subtle">{{
                  t("keys.accountBalance")
                }}</span
                ><span v-if="key.mode === 'rolling'" class="key-table-subtle"
                  >{{
                    windowOptions.find((option) => option.value === key.window)
                      ?.label
                  }}{{ t("keys.rolling") }}</span
                >
              </td>
              <td class="key-tier-column">{{ tierLabel(key.tier) }}</td>
              <td
                class="key-expiry-cell"
                :data-expiry-label="t('keys.expiry') + ' · '"
              >
                <span>{{
                  key.expires
                    ? key.expires.replaceAll("-", "/")
                    : t("keys.noExpiry")
                }}</span>
              </td>
              <td class="key-actions">
                <div class="key-action-group">
                  <button class="text-button" @click="inspect(key)">
                    {{ t("keys.details") }}</button
                  ><DropdownMenuRoot
                    ><DropdownMenuTrigger
                      class="icon-button"
                      :aria-label="key.name + t('keys.moreSuffix')"
                      ><OrbitIcon name="more" :size="17" /></DropdownMenuTrigger
                    ><DropdownMenuPortal
                      ><DropdownMenuContent
                        class="key-menu"
                        align="end"
                        :side-offset="5"
                        :collision-padding="12"
                        ><DropdownMenuItem
                          class="key-menu-item"
                          :disabled="
                            readOnly ||
                            !canSelect(key) ||
                            (key.id === defaultId && api.page?.selected_ready)
                          "
                          @select="choose(key)"
                          ><OrbitIcon name="check" :size="15" />{{
                            t("keys.setDefault")
                          }}</DropdownMenuItem
                        ><DropdownMenuItem
                          class="key-menu-item"
                          :disabled="
                            readOnly ||
                            ['revoked', 'retired'].includes(key.status)
                          "
                          @select="edit(key)"
                          ><OrbitIcon name="edit" :size="15" />{{
                            t("keys.editKey")
                          }}</DropdownMenuItem
                        ><DropdownMenuItem
                          class="key-menu-item"
                          :disabled="
                            !api.page?.can_reveal || key.status === 'retired'
                          "
                          @select="openReveal(key)"
                          ><OrbitIcon name="key" :size="15" />{{
                            t("keys.viewCopy")
                          }}</DropdownMenuItem
                        ><DropdownMenuItem
                          class="key-menu-item"
                          :disabled="
                            readOnly ||
                            ['revoked', 'retired', 'expired'].includes(
                              key.status,
                            )
                          "
                          @select="requestAction('rotate', key)"
                          ><OrbitIcon name="refresh" :size="15" />{{
                            t("keys.rotate")
                          }}</DropdownMenuItem
                        ><DropdownMenuItem
                          class="key-menu-item"
                          :disabled="
                            readOnly ||
                            ['revoked', 'retired'].includes(key.status)
                          "
                          @select="requestAction('reset', key)"
                          ><OrbitIcon name="history" :size="15" />{{
                            t("keys.reset")
                          }}</DropdownMenuItem
                        ><DropdownMenuSeparator
                          class="key-menu-separator"
                        /><DropdownMenuItem
                          class="key-menu-item danger"
                          :disabled="
                            readOnly ||
                            ['revoked', 'retired'].includes(key.status)
                          "
                          @select="requestAction('revoke', key)"
                          ><OrbitIcon name="close" :size="15" />{{
                            t("keys.revoke")
                          }}</DropdownMenuItem
                        ><DropdownMenuItem
                          class="key-menu-item danger"
                          :disabled="readOnly"
                          @select="requestAction('delete', key)"
                          ><OrbitIcon name="trash" :size="15" />{{
                            t("keys.delete")
                          }}</DropdownMenuItem
                        ></DropdownMenuContent
                      ></DropdownMenuPortal
                    ></DropdownMenuRoot
                  >
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="key-empty">
        <div class="key-empty-symbol">
          <OrbitIcon name="key" :size="27" />
        </div>
        <h3>
          {{
            api.page?.total || search || filter !== "all"
              ? t("keys.noMatch")
              : t("keys.first")
          }}
        </h3>
        <p>
          {{
            api.page?.total || search || filter !== "all"
              ? t("keys.searchHint")
              : t("keys.firstHint")
          }}
        </p>
        <button
          v-if="search || filter !== 'all'"
          class="button secondary"
          @click="
            search = '';
            filter = 'all';
          "
        >
          {{ t("keys.clear") }}</button
        ><button
          v-else
          class="button primary"
          :disabled="readOnly"
          @click="edit()"
        >
          <OrbitIcon name="plus" :size="15" />{{ t("keys.create") }}
        </button>
      </div>
      <div v-if="filtered.length" class="key-table-footer">
        <span
          >{{ t("keys.showing") }} {{ filtered.length }} {{ t("keys.of") }}
          {{ api.page.total }} {{ t("keys.count") }}</span
        >
        <div v-if="api.page.last_page > 1" class="key-pagination">
          <button
            class="text-button"
            :disabled="api.page.page <= 1 || api.loading"
            @click="
              api.number--;
              api.load();
            "
          >
            {{ t("keys.previous") }}
          </button>
          <span>{{ api.page.page }} / {{ api.page.last_page }}</span>
          <button
            class="text-button"
            :disabled="api.page.page >= api.page.last_page || api.loading"
            @click="
              api.number++;
              api.load();
            "
          >
            {{ t("keys.next") }}
          </button>
        </div>
        <span>{{ t("keys.currency") }} {{ api.page.currency }}</span>
      </div>
    </section>
  </div>
  <ApiKeyPickerDialog v-model:open="switchOpen" />

  <OrbitDialog
    v-model:open="detailsOpen"
    :title="detail?.name ?? t('keys.detailTitle')"
    :description="t('keys.detailHint')"
    ><template v-if="detail"
      ><div class="key-detail-secret">
        <code>{{ detail.prefix }}</code
        ><button
          class="text-button"
          :disabled="!api.page?.can_reveal || detail.status === 'retired'"
          @click="openReveal(detail)"
        >
          {{ t("keys.viewCopy") }}<OrbitIcon name="external" :size="13" />
        </button>
      </div>
      <dl class="key-detail-grid">
        <div>
          <dt>{{ t("keys.status") }}</dt>
          <dd>
            <StatusBadge
              :tone="tone(detail.status)"
              :text="statusLabels[detail.status]"
            />
          </dd>
        </div>
        <div>
          <dt>{{ t("keys.tier") }}</dt>
          <dd>{{ tierLabel(detail.tier) }}</dd>
        </div>
        <div>
          <dt>{{ t("keys.expiry") }}</dt>
          <dd>{{ detail.expires || t("keys.noExpiry") }}</dd>
        </div>
        <div>
          <dt>{{ t("keys.lastUsed") }}</dt>
          <dd>{{ detail.lastUsed }}</dd>
        </div>
        <div>
          <dt>{{ t("keys.createdAt") }}</dt>
          <dd>{{ detail.created }}</dd>
        </div>
        <div>
          <dt>{{ t("keys.limitMode") }}</dt>
          <dd>
            {{
              detail.mode === "rolling"
                ? `${windowOptions.find((option) => option.value === detail?.window)?.label} ${t("keys.rolling")}`
                : detail.mode === "unlimited"
                  ? t("keys.unlimited")
                  : t("keys.cumulative")
            }}
          </dd>
        </div>
      </dl>
      <div class="key-detail-spending">
        <span>{{ t("keys.used") }}</span
        ><strong
          >{{ money(detail.used)
          }}<small
            >/
            {{
              detail.limit === null ? t("keys.unlimited") : money(detail.limit)
            }}</small
          ></strong
        >
        <div
          v-if="detail.limit"
          class="key-progress"
          :class="{ exhausted: detail.status === 'spending_exhausted' }"
        >
          <span :style="{ width: `${percent(detail)}%` }" />
        </div>
        <button
          class="text-button"
          @click="
            detailsOpen = false;
            router.push({ path: '/usage', query: { api_key_id: detail.id } });
          "
        >
          {{ t("keys.usage") }}<OrbitIcon name="arrow" :size="13" />
        </button>
      </div>
      <div class="key-detail-bindings">
        <p class="key-caption">{{ t("keys.localBindings") }}</p>
        <p v-if="detail.id === defaultId">
          <OrbitIcon name="check" :size="14" />{{ t("keys.defaultKey") }}
        </p>
        <p v-if="detail.id === bound.codex">
          <OrbitIcon name="window" :size="14" />Codex
        </p>
        <p v-if="detail.id === bound.claude">
          <OrbitIcon name="window" :size="14" />Claude Desktop
        </p>
        <p
          v-if="
            detail.id !== defaultId &&
            detail.id !== bound.codex &&
            detail.id !== bound.claude
          "
          class="muted"
        >
          {{ t("keys.noBindings") }}
        </p>
      </div>
      <div class="dialog-actions">
        <button
          class="button secondary"
          :disabled="readOnly || ['revoked', 'retired'].includes(detail.status)"
          @click="edit(detail)"
        >
          <OrbitIcon name="edit" :size="14" />{{ t("keys.edit") }}</button
        ><button
          class="button primary"
          :disabled="
            readOnly ||
            !canSelect(detail) ||
            (detail.id === defaultId && api.page?.selected_ready)
          "
          @click="
            choose(detail);
            detailsOpen = false;
          "
        >
          {{
            detail.id === defaultId
              ? t("keys.currentDefault")
              : t("keys.setDefault")
          }}
        </button>
      </div></template
    ></OrbitDialog
  >

  <OrbitDialog
    v-model:open="editorOpen"
    :title="editingId ? t('keys.editKey') : t('keys.createTitle')"
    :description="t('keys.createHint')"
    ><form class="key-form" @submit.prevent="save">
      <div class="field">
        <label class="field-label" for="key-name">{{ t("keys.name") }}</label
        ><input
          id="key-name"
          v-model="form.name"
          :placeholder="t('keys.namePlaceholder')"
          maxlength="100"
          required
        />
      </div>
      <div class="key-form-columns">
        <div class="field">
          <label class="field-label" for="key-expiry">{{
            t("keys.expiry")
          }}</label
          ><input
            id="key-expiry"
            v-model="form.expires"
            type="date"
            :min="editingKey ? undefined : localDate(new Date())"
          />
          <p class="field-hint">{{ t("keys.expiryHint") }}</p>
        </div>
        <div class="field">
          <label class="field-label" for="key-tier">{{ t("keys.tier") }}</label
          ><OrbitSelect
            id="key-tier"
            v-model="form.tier"
            :options="tierOptions"
          />
          <p class="field-hint">{{ t("keys.tierHint") }}</p>
        </div>
      </div>
      <div class="key-form-limit">
        <div class="field">
          <label class="field-label" for="key-mode">{{
            t("keys.amountLimit")
          }}</label
          ><OrbitSelect
            id="key-mode"
            v-model="form.mode"
            :options="[
              { value: 'unlimited', label: t('keys.unlimited') },
              { value: 'cumulative', label: t('keys.cumulative') },
              { value: 'rolling', label: t('keys.rolling') },
            ]"
          />
        </div>
        <div v-if="form.mode !== 'unlimited'" class="key-form-columns">
          <div class="field">
            <label class="field-label" for="key-limit"
              >{{ t("keys.limitAmount") }}
              <span class="key-unit">{{ api.page?.currency }}</span></label
            ><input
              id="key-limit"
              v-model="form.limit"
              type="text"
              inputmode="decimal"
              pattern="[0-9]+([.][0-9]{1,8})?"
              placeholder="50.00"
              required
            />
          </div>
          <div v-if="form.mode === 'rolling'" class="field">
            <label class="field-label" for="key-window">{{
              t("keys.rolling")
            }}</label
            ><OrbitSelect
              id="key-window"
              v-model="form.window"
              :options="windowOptions"
            />
          </div>
        </div>
        <p class="field-hint">
          {{
            form.mode === "unlimited"
              ? t("keys.unlimitedHint")
              : form.mode === "rolling"
                ? t("keys.rollingHint")
                : t("keys.cumulativeHint")
          }}
        </p>
      </div>
      <label v-if="!editingId" class="key-create-check"
        ><input v-model="form.use" type="checkbox" /><span>{{
          t("keys.selectAfterCreate")
        }}</span></label
      >
      <div class="dialog-actions">
        <button
          type="button"
          class="button secondary"
          @click="editorOpen = false"
        >
          {{ t("keys.cancel") }}</button
        ><button type="submit" class="button primary" :disabled="readOnly">
          {{
            editingId
              ? t("keys.save")
              : form.use
                ? t("keys.createUse")
                : t("keys.create")
          }}
        </button>
      </div>
    </form></OrbitDialog
  >

  <OrbitDialog
    v-model:open="confirmOpen"
    :title="pendingAction.title"
    :description="pendingAction.description"
    ><div v-if="pendingKey" class="key-confirm-target">
      <OrbitIcon name="key" :size="20" />
      <div>
        <strong>{{ pendingKey.name }}</strong
        ><span class="key-prefix mono">{{ pendingKey.prefix }}</span>
      </div>
    </div>
    <p
      v-if="pending?.action === 'reset' && pendingKey"
      class="key-confirm-note"
    >
      {{ t("keys.currentUsed") }} {{ money(pendingKey.used)
      }}{{ t("keys.resetZero") }} {{ money("0.00000000") }}。
    </p>
    <div class="dialog-actions">
      <button class="button secondary" @click="confirmOpen = false">
        {{ t("keys.cancel") }}</button
      ><button
        class="button"
        :class="pending?.action === 'reset' ? 'primary' : 'danger-button'"
        :disabled="readOnly"
        @click="confirm"
      >
        {{ pendingAction.verb }}
      </button>
    </div></OrbitDialog
  >

  <OrbitDialog
    v-model:open="revealOpen"
    :title="t('keys.revealTitle')"
    :description="t('keys.revealHint')"
    ><p class="key-secret-label">{{ revealKey?.name }}</p>
    <div class="key-reveal-value mono">
      {{ revealed ? revealedToken : "orv_sk_••••••••••••••••••••••••••••" }}
    </div>
    <p class="field-hint">{{ t("keys.hideHint") }}</p>
    <div class="dialog-actions">
      <button
        class="button secondary"
        :disabled="revealing"
        @click="revealed ? hideSecret() : reveal()"
      >
        {{ revealed ? t("keys.hide") : t("keys.reveal") }}</button
      ><button class="button primary" @click="copy">
        <OrbitIcon name="copy" :size="15" />{{ t("keys.copy") }}
      </button>
    </div></OrbitDialog
  >
</template>
