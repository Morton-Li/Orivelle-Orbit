<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import { t, type MessageKey } from "../shared/i18n";
import { nodeName } from "../shared/format";
import type {
  ClaudeRestoreCoordination,
  ClaudeModelMapping,
  ClientKind,
  ConfigPreview,
} from "../shared/contracts";
import { CAPABILITIES } from "../shared/contracts";
import ClientRow from "../components/ClientRow.vue";
import ConfigReviewDialog from "../components/ConfigReviewDialog.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
import { useNodeLatency } from "../composables/useNodeLatency";
const store = useOrbitStore();
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
const previewing = ref<Record<ClientKind, boolean>>({
  codex: false,
  claude: false,
});
let previewGeneration = 0;
onUnmounted(() => {
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
const claudeOptions = computed(() =>
  claudeModels.value.map((model) => ({ value: model.id, label: model.label })),
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
    return "access.waitCredential";
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
watch(
  () => store.scope,
  () => {
    previewGeneration++;
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
  try {
    const result = await store.perform(
      `preview-${client}`,
      "preview_client_config",
      {
        connection_id: connection.id,
        expected_revision: connection.revision,
        client,
        ...(mapping ? { claude_mapping: mapping } : {}),
      },
    );
    if (!result || !current()) return;
    // Native preview refreshes Panel's catalogue. Synchronize that authoritative
    // model revision before the dialog can permanently retire a fresh preview.
    const noticeBeforeRead = store.notice;
    const synchronized = await store.loadModels();
    if (!current()) return;
    if (
      !synchronized &&
      store.notice !== noticeBeforeRead &&
      store.notice?.tone === "error"
    )
      return;
    const active = store.activeConnection;
    if (
      !synchronized ||
      result.operation !== "apply" ||
      result.client !== client ||
      !store.ready ||
      (mapping &&
        (result.model_id !== mapping.sonnet ||
          !mappingReady.value ||
          claudeMappings.some(
            ({ key }) => mapping[key] !== selectedMapping.value[key],
          ))) ||
      result.context.connection_id !== active?.id ||
      result.context.account_id !== active.account_id ||
      result.context.connection_revision !== active.revision ||
      result.context.credential_revision !== active.credential_revision ||
      result.context.node_id !== active.node_id ||
      !store.models.some(
        (model) =>
          model.id === result.model_id &&
          model.revision === result.context.model_revision,
      )
    ) {
      store.notify(t("access.previewExpired"));
      return;
    }
    preview.value = result;
  } finally {
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
</script>
<template>
  <section class="page-heading">
    <h1>{{ t("access.title") }}</h1>
    <p class="page-description">{{ t("access.description") }}</p>
  </section>
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
        <template v-if="switchSnapshot(client.client)">
          <p class="muted">{{ t("access.switchHint") }}</p>
          <button
            class="button secondary"
            :disabled="
              switchSnapshot(client.client)?.state === 'unavailable' ||
              !configurationAllowed(client.client) ||
              store.busy.has('preview-restore')
            "
            @click="restore(switchSnapshot(client.client)!.id)"
          >
            <OrbitIcon name="history" />{{ t("access.switchBack") }}
          </button>
          <button
            v-if="client.client === 'claude'"
            class="text-button"
            :disabled="
              switchSnapshot(client.client)?.state === 'unavailable' ||
              !configurationAllowed(client.client) ||
              store.busy.has('preview-restore')
            "
            @click="
              restore(
                switchSnapshot(client.client)!.id,
                'claude_main_preferences_v1',
              )
            "
          >
            {{ t("access.switchBackPreservingPreferences") }}
          </button>
        </template>
        <template v-else>
          <p v-if="client.client === 'codex'" class="muted">
            {{ t("access.allCodexModels") }}
          </p>
          <section
            v-else
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
                  v-model="selectedMapping[entry.key]"
                  :options="claudeOptions"
                  :placeholder="
                    t(
                      store.modelsLoading
                        ? 'common.loading'
                        : 'access.selectModel',
                    )
                  "
                  :disabled="
                    store.modelsLoading ||
                    !claudeOptions.length ||
                    previewing.claude
                  "
                />
              </div>
            </div>
          </section>
          <button
            class="button primary"
            :class="{ 'claude-connect': client.client === 'claude' }"
            :aria-describedby="
              connectReason(client.client)
                ? `connect-reason-${client.client}`
                : undefined
            "
            :disabled="
              !store.ready ||
              !previewAllowed(client.client) ||
              (client.client === 'claude'
                ? !mappingReady
                : !codexModelsAvailable) ||
              store.modelsLoading ||
              previewing[client.client] ||
              store.busy.has(`preview-${client.client}`)
            "
            @click="requestPreview(client.client)"
          >
            <OrbitIcon name="access" />{{ t("access.connectOrbit") }}
          </button>
          <p
            v-if="connectReason(client.client)"
            :id="`connect-reason-${client.client}`"
            class="field-hint"
            role="status"
          >
            {{ t(connectReason(client.client)!) }}
          </p>
        </template>
      </div>
    </article>
  </section>
  <ConfigReviewDialog
    :preview="preview"
    @close="preview = null"
    @applied="preview = null"
  />
</template>
