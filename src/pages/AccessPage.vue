<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { useApiKeysStore } from "../stores/api-keys";
import { useOrbitStore } from "../stores/orbit";
import { t, type MessageKey } from "../shared/i18n";
import { call } from "../shared/bridge";
import {
  matchesReviewConnection,
  matchesReviewModel,
} from "../shared/config-review";
import { nodeName } from "../shared/format";
import type {
  ClaudeRestoreCoordination,
  ClaudeModelMapping,
  ClientKind,
  ConfigPreview,
  ConfigResult,
} from "../shared/contracts";
import { CAPABILITIES } from "../shared/contracts";
import ClientRow from "../components/ClientRow.vue";
import ConfigReviewDialog from "../components/ConfigReviewDialog.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
import { useNodeLatency } from "../composables/useNodeLatency";
const store = useOrbitStore();
const apiKeys = useApiKeysStore();
const route = useRoute();
const updatingClient = ref<ClientKind | null>(null);
const latency = useNodeLatency();
const nodeOptions = computed(() =>
  store.state.nodes.map((node, index) => ({
    value: node.id,
    label: nodeName(node, index),
    detail: latency.label(node),
    disabled: !node.available,
  })),
);
const claudeMappings = [
  { key: "sonnet", label: "access.mappingSonnet" },
  { key: "opus", label: "access.mappingOpus" },
  { key: "haiku", label: "access.mappingHaiku" },
] as const;
const selectedMapping = ref<ClaudeModelMapping>({
  sonnet: "",
  opus: "",
  haiku: "",
});
const preview = ref<ConfigPreview | null>(null);
const codexReviewOpen = ref(false);
const reviewError = ref("");
function discard(prepared: ConfigPreview | null | undefined) {
  if (prepared?.client === "codex" && prepared.expires_at === null) {
    void call("discard_config_preview", { preview_id: prepared.id }).catch(
      () => {},
    );
  }
}
const previewing = ref<Record<ClientKind, boolean>>({
  codex: false,
  claude: false,
});
let previewGeneration = 0;
onUnmounted(() => {
  discard(preview.value);
  previewGeneration++;
});
function configurationAllowed(kind: ClientKind): boolean {
  const client = store.state.clients.find((item) => item.client === kind);
  return !!(
    client?.installed &&
    client.platform_supported &&
    client.managed !== "yes" &&
    client.configuration_access === "allowed" &&
    !!client.configuration_access_revision
  );
}
function previewAllowed(kind: ClientKind): boolean {
  const client = store.state.clients.find((item) => item.client === kind);
  return (
    configurationAllowed(kind) ||
    !!(
      kind === "codex" &&
      client?.installed &&
      client.platform_supported &&
      client.managed !== "yes" &&
      client.configuration_access === "unverified"
    ) ||
    !!(
      kind === "claude" &&
      client?.installed &&
      client.platform_supported &&
      client.managed === "no"
    )
  );
}
function switchSnapshot(kind: ClientKind) {
  return store.state.switch_snapshots.find(
    (snapshot) => snapshot.client === kind,
  );
}
const claudeModels = computed(() =>
  store.textModels.filter(
    (model) =>
      model.provider_id === store.activeConnection?.provider_id &&
      model.capabilities.includes(CAPABILITIES.messages),
  ),
);
const boundClaudeMapping = computed(() => {
  const snapshot = switchSnapshot("claude");
  if (
    !snapshot ||
    snapshot.account_id !== store.account?.id ||
    snapshot.panel_origin !== store.activeConnection?.panel_origin
  )
    return null;
  return (
    apiKeys.page?.bindings.find((binding) => binding.client === "claude")
      ?.claude_mapping ?? null
  );
});
const displayedClaudeMapping = computed(() =>
  switchSnapshot("claude")
    ? (boundClaudeMapping.value ?? { sonnet: "", opus: "", haiku: "" })
    : selectedMapping.value,
);
const claudeOptions = computed(() => {
  const options = claudeModels.value.map((model) => ({
    value: model.id,
    label: model.label,
  }));
  if (boundClaudeMapping.value) {
    for (const id of Object.values(boundClaudeMapping.value)) {
      if (id && !options.some((option) => option.value === id))
        options.push({ value: id, label: id });
    }
  }
  return options;
});
function selectClaudeModel(role: keyof ClaudeModelMapping, value: string) {
  if (!switchSnapshot("claude")) selectedMapping.value[role] = value;
}
watch(
  boundClaudeMapping,
  (mapping) => {
    if (mapping) selectedMapping.value = { ...mapping };
  },
  { immediate: true },
);
const codexModelsAvailable = computed(() =>
  store.textModels.some(
    (model) =>
      model.provider_id === store.activeConnection?.provider_id &&
      model.capabilities.includes(CAPABILITIES.responses),
  ),
);
const mappingReady = computed(() =>
  claudeMappings.every(({ key }) =>
    claudeModels.value.some((model) => model.id === selectedMapping.value[key]),
  ),
);
function connectReason(kind: ClientKind): MessageKey | null {
  const client = store.state.clients.find((item) => item.client === kind);
  if (!store.initialized) return "access.waitDetection";
  if (!store.account) return "access.connectAccountFirst";
  if (store.activeConnection?.permission_state === "readonly")
    return "access.readonly";
  if (store.state.session.inference_credential_status !== "ready")
    return apiKeys.page ? "access.chooseKeyFirst" : "access.waitCredential";
  if (!store.activeConnection?.node_id) return "access.chooseNodeFirst";
  if (store.state.maintenance_active) return "access.waitMaintenance";
  if (!client?.installed) return "access.installClientFirst";
  if (!client.platform_supported) return "access.platformUnavailable";
  if (client.managed === "yes" || client.configuration_access === "blocked")
    return "client.configurationBlocked";
  if (!previewAllowed(kind)) return "access.checkConfiguration";
  if (store.modelsLoading) return "access.waitModels";
  if (kind === "codex" && !codexModelsAvailable.value)
    return "access.noCompatibleModels";
  if (kind === "claude" && !claudeModels.value.length)
    return "access.noCompatibleModels";
  if (kind === "claude" && !mappingReady.value) return "access.completeMapping";
  if (!store.ready) return "access.waitConnection";
  if (previewing.value[kind] || store.busy.has(`preview-${kind}`))
    return "access.preparingPreview";
  return null;
}
function connectHint(kind: ClientKind): MessageKey | null {
  const reason = connectReason(kind);
  return reason === "access.chooseKeyFirst" ? null : reason;
}
watch(
  () => store.scope,
  () => {
    previewGeneration++;
    if (codexReviewOpen.value) reviewError.value = t("access.scopeChanged");
    selectedMapping.value = { sonnet: "", opus: "", haiku: "" };
  },
  { flush: "sync" },
);
watch(
  claudeModels,
  (models) => {
    for (const { key } of claudeMappings) {
      if (!models.some((model) => model.id === selectedMapping.value[key]))
        selectedMapping.value[key] = "";
      if (!selectedMapping.value[key]) {
        const matches = models.filter((model) =>
          new RegExp(`\\b${key}\\b`, "i").test(`${model.id} ${model.label}`),
        );
        if (matches.length === 1) selectedMapping.value[key] = matches[0]!.id;
      }
    }
  },
  { immediate: true },
);
async function requestPreview(client: ClientKind) {
  const reason = connectReason(client);
  if (reason && codexReviewOpen.value && client === "codex") {
    reviewError.value = t(reason);
    return;
  }
  if (reason === "access.chooseKeyFirst") {
    store.notify(t(reason), "warning");
    return;
  }
  const connection = store.activeConnection;
  if (
    !connection ||
    switchSnapshot(client) ||
    (client === "claude" ? !mappingReady.value : !codexModelsAvailable.value) ||
    !previewAllowed(client) ||
    previewing.value[client]
  )
    return;
  const scope = store.scope;
  const origin = connection.panel_origin;
  const mapping =
    client === "claude" ? { ...selectedMapping.value } : undefined;
  const generation = ++previewGeneration;
  const current = () =>
    generation === previewGeneration &&
    scope === store.scope &&
    origin === store.activeConnection?.panel_origin;
  previewing.value[client] = true;
  let prepared: ConfigPreview | undefined;
  if (client === "codex") {
    discard(preview.value);
    preview.value = null;
    reviewError.value = "";
    codexReviewOpen.value = true;
  }
  try {
    if (client === "codex") {
      await nextTick();
      if (!current()) return;
    }
    const result = await store.perform(
      `preview-${client}`,
      "preview_client_config",
      {
        connection_id: connection.id,
        expected_revision: connection.revision,
        client,
        ...(mapping ? { claude_mapping: mapping } : {}),
      },
      {
        onError: (error) => {
          if (!current()) return;
          const message = store.report(error);
          if (client === "codex") reviewError.value = message;
        },
      },
    );
    prepared = result;
    if (!current()) return;
    if (!result) {
      if (client === "codex" && !reviewError.value)
        reviewError.value = t(
          store.state.maintenance_active
            ? "common.updateMaintenance"
            : "common.failed",
        );
      return;
    }
    // Native preview refreshes Panel's catalogue. Synchronize that authoritative
    // model revision before the dialog can permanently retire a fresh preview.
    const noticeBeforeRead = store.notice;
    const synchronized =
      (client === "codex" &&
        result.expires_at === null &&
        matchesReviewModel(result, store.models)) ||
      (await store.loadModels());
    if (!current()) return;
    if (
      !synchronized &&
      store.notice !== noticeBeforeRead &&
      store.notice?.tone === "error"
    ) {
      if (client === "codex") reviewError.value = store.notice.text;
      return;
    }
    const sameConnection = matchesReviewConnection(
      result,
      store.activeConnection,
    );
    if (
      !synchronized ||
      result.operation !== "apply" ||
      result.client !== client ||
      (!store.ready && (client !== "codex" || result.expires_at !== null)) ||
      (mapping &&
        (result.model_id !== mapping.sonnet ||
          !mappingReady.value ||
          claudeMappings.some(
            ({ key }) => mapping[key] !== selectedMapping.value[key],
          ))) ||
      !sameConnection ||
      !matchesReviewModel(result, store.models)
    ) {
      const message =
        client === "codex"
          ? t(
              sameConnection
                ? "access.reviewModelsChanged"
                : "access.scopeChanged",
            )
          : t("access.previewExpired");
      if (client === "codex") reviewError.value = message;
      else store.notify(message);
      return;
    }
    preview.value = result;
    prepared = undefined;
  } finally {
    discard(prepared);
    previewing.value[client] = false;
  }
}
async function restore(id: string, coordination?: ClaudeRestoreCoordination) {
  const backup = store.state.switch_snapshots.find((item) => item.id === id);
  if (
    !backup ||
    backup.state === "unavailable" ||
    (coordination && backup.client !== "claude") ||
    !configurationAllowed(backup.client) ||
    store.busy.has("preview-restore")
  )
    return;
  const expected = { ...backup };
  const generation = ++previewGeneration;
  const result = await store.perform("preview-restore", "preview_restore", {
    snapshot_id: id,
    ...(coordination ? { coordination } : {}),
  });
  if (!result || generation !== previewGeneration) return;
  if (
    result.operation !== "restore" ||
    (result.restore_coordination ?? undefined) !== coordination ||
    result.restore_source?.snapshot_id !== id ||
    result.client !== expected.client ||
    result.restore_owner.account_id !== expected.account_id ||
    result.restore_owner.panel_origin !== expected.panel_origin ||
    result.restore_owner.installation_id !== expected.installation_id ||
    result.restore_source.expires_at !== expected.expires_at ||
    result.paths.length !== expected.file_count
  ) {
    store.notify(t("access.restorePreviewExpired"));
    return;
  }
  preview.value = result;
}
async function configurationApplied(result: ConfigResult) {
  codexReviewOpen.value = false;
  reviewError.value = "";
  preview.value = null;
  if (updatingClient.value === result.client && result.state === "restored") {
    await store.loadModels();
    await requestPreview(result.client);
  } else if (result.state === "saved") {
    updatingClient.value = null;
    apiKeys.updateMapping = null;
    await apiKeys.load();
  }
}
function closePreview() {
  previewGeneration++;
  discard(preview.value);
  codexReviewOpen.value = false;
  reviewError.value = "";
  preview.value = null;
  updatingClient.value = null;
  apiKeys.updateMapping = null;
}
onMounted(async () => {
  await apiKeys.load();
  const client = route?.query.update;
  if (
    (client !== "codex" && client !== "claude") ||
    apiKeys.updateMapping?.client !== client
  )
    return;
  const backup = switchSnapshot(client);
  if (
    !backup ||
    backup.account_id !== store.account?.id ||
    backup.panel_origin !== store.activeConnection?.panel_origin
  )
    return;
  updatingClient.value = client;
  if (client === "claude" && apiKeys.updateMapping.mapping)
    selectedMapping.value = { ...apiKeys.updateMapping.mapping };
  await restore(
    backup.id,
    client === "claude" ? "claude_main_settings_v2" : undefined,
  );
});
</script>
<template>
  <section class="page-heading">
    <h1>{{ t("access.title") }}</h1>
    <p class="page-description">{{ t("access.description") }}</p>
  </section>
  <p v-if="updatingClient" class="info-box" role="status">
    {{ t("access.updatingKey") }}
  </p>
  <section class="connection-settings" :aria-label="t('access.connection')">
    <div class="access-account">
      <span class="account-avatar" aria-hidden="true"
        ><OrbitIcon name="account"
      /></span>
      <div class="account-summary">
        <p class="field-label">{{ t("access.connection") }}</p>
        <p class="access-account-name">
          {{ store.account?.label ?? t("access.noConnection") }}
        </p>
      </div>
      <button
        v-if="store.canLogin"
        class="button secondary"
        @click="store.loginOpen = true"
      >
        {{ t("action.login") }}
      </button>
    </div>
    <div class="field">
      <div class="field-heading">
        <label for="access-node" class="field-label">{{
          t("access.node")
        }}</label>
        <button
          type="button"
          class="button ghost node-latency-refresh"
          :disabled="!latency.active.value || latency.refreshing.value"
          :aria-busy="latency.refreshing.value"
          @click="latency.refresh"
        >
          <OrbitIcon name="refresh" :size="13" />
          {{
            t(
              latency.refreshing.value
                ? "access.latencyRefreshing"
                : "access.latencyRefresh",
            )
          }}
        </button>
      </div>
      <OrbitSelect
        id="access-node"
        :model-value="store.activeConnection?.node_id ?? ''"
        :disabled="
          !store.activeConnection ||
          store.busy.has('node') ||
          !store.state.nodes.some((node) => node.available)
        "
        :placeholder="
          t(
            store.state.nodes.some((node) => node.available)
              ? 'access.selectNode'
              : 'access.noNode',
          )
        "
        :options="nodeOptions"
        @update:model-value="store.selectNode"
      />
    </div>
  </section>
  <p
    v-if="store.activeConnection?.permission_state === 'readonly'"
    class="info-box"
  >
    {{ t("access.readonly") }}
  </p>
  <section class="access-client-list">
    <article
      v-for="client in store.state.clients"
      :id="`client-${client.client}`"
      :key="client.client"
      class="access-client"
    >
      <ClientRow
        :client="client"
        :connection="store.activeConnection"
        :detected="store.initialized"
        compact
      />
      <div class="access-client-controls">
        <section
          v-if="client.client === 'claude'"
          class="claude-model-config"
          aria-labelledby="claude-model-heading"
        >
          <h3 id="claude-model-heading">{{ t("access.modelMapping") }}</h3>
          <p class="muted">{{ t("access.claudeMappingHint") }}</p>
          <div class="claude-mappings">
            <div
              v-for="entry in claudeMappings"
              :key="entry.key"
              class="claude-mapping"
            >
              <label :for="`claude-${entry.key}`" class="field-label">{{
                t(entry.label)
              }}</label>
              <OrbitSelect
                :id="`claude-${entry.key}`"
                :model-value="displayedClaudeMapping[entry.key]"
                :options="claudeOptions"
                :placeholder="
                  t(
                    store.modelsLoading
                      ? 'common.loading'
                      : 'access.selectModel',
                  )
                "
                :disabled="
                  !!switchSnapshot('claude') ||
                  store.modelsLoading ||
                  !claudeOptions.length ||
                  previewing.claude
                "
                @update:model-value="selectClaudeModel(entry.key, $event)"
              />
            </div>
          </div>
        </section>
        <template v-if="switchSnapshot(client.client)">
          <button
            class="button secondary access-client-action"
            :disabled="
              switchSnapshot(client.client)?.state === 'unavailable' ||
              !configurationAllowed(client.client) ||
              store.busy.has('preview-restore')
            "
            @click="
              restore(
                switchSnapshot(client.client)!.id,
                client.client === 'claude'
                  ? 'claude_main_settings_v2'
                  : undefined,
              )
            "
          >
            <OrbitIcon name="history" />{{ t("access.cancelAccess") }}
          </button>
        </template>
        <template v-else>
          <p v-if="client.client === 'codex'" class="muted">
            {{ t("access.allCodexModels") }}
          </p>
          <div
            class="access-connect-actions"
            :class="{ 'claude-connect-actions': client.client === 'claude' }"
          >
            <button
              class="button primary access-client-action"
              :aria-describedby="
                connectHint(client.client)
                  ? `connect-reason-${client.client}`
                  : undefined
              "
              :disabled="!!connectHint(client.client)"
              @click="requestPreview(client.client)"
            >
              <OrbitIcon name="access" />{{ t("access.connectOrbit") }}
            </button>
            <p
              v-if="connectHint(client.client)"
              :id="`connect-reason-${client.client}`"
              class="field-hint"
              role="status"
            >
              {{ t(connectHint(client.client)!) }}
            </p>
          </div>
        </template>
      </div>
    </article>
  </section>
  <ConfigReviewDialog
    :preview="preview"
    :codex-review-open="codexReviewOpen"
    :checking="previewing.codex"
    :error="reviewError"
    @retry="requestPreview('codex')"
    @close="closePreview"
    @applied="configurationApplied"
  />
</template>
