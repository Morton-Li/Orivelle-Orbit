<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import type { ConfigPreview } from "../shared/contracts";
import { t } from "../shared/i18n";
import {
  configPolicyMessage,
  nodeName,
  redactServiceUrls,
} from "../shared/format";
import OrbitDialog from "./OrbitDialog.vue";
import OrbitIcon from "./OrbitIcon.vue";
const props = defineProps<{ preview: ConfigPreview | null }>();
const emit = defineEmits<{ close: []; applied: [] }>();
const store = useOrbitStore();
const now = ref(Date.now());
const rejectedPreviewIds = ref(new Set<string>());
const targetClient = computed(() =>
  store.state.clients.find((item) => item.client === props.preview?.client),
);
const previewNodeName = computed(() => {
  const preview = props.preview;
  if (preview?.operation !== "apply") return "";
  const index = store.state.nodes.findIndex(
    (node) => node.id === preview.context.node_id,
  );
  const node = store.state.nodes[index];
  return node ? nodeName(node, index) : t("access.node");
});
const timer = window.setInterval(() => {
  now.value = Date.now();
}, 1000);
onUnmounted(() => window.clearInterval(timer));
const current = computed(() => {
  const preview = props.preview;
  if (!preview || rejectedPreviewIds.value.has(preview.id)) return false;
  const client = targetClient.value;
  const expiresAt = Date.parse(preview.expires_at);
  if (
    !client?.installed ||
    !client.platform_supported ||
    client.version !== preview.target_version ||
    !Number.isFinite(expiresAt) ||
    expiresAt <= now.value
  )
    return false;
  const diagnosticExport =
    preview.client === "claude" &&
    preview.operation === "apply" &&
    preview.can_export_import &&
    !preview.can_apply;
  if (diagnosticExport) {
    if (client.managed !== "no") return false;
  } else if (
    client.managed === "yes" ||
    client.configuration_access !== "allowed" ||
    !client.configuration_access_revision
  )
    return false;
  if (preview.client === "codex") {
    const target = preview.codex_configuration_target;
    if (
      !target?.revision ||
      !target.cli_version ||
      target.revision !== client.configuration_access_revision
    )
      return false;
    if (preview.operation === "apply" && !preview.codex_catalog) return false;
  } else if (!diagnosticExport) {
    const target = preview.claude_configuration_target;
    if (
      !target?.revision ||
      !target.app_version ||
      target.app_version !== client.version ||
      target.revision !== client.configuration_access_revision
    )
      return false;
  }
  if (preview.operation === "restore") {
    if (
      preview.restore_coordination != null &&
      (preview.client !== "claude" ||
        preview.restore_coordination !== "claude_main_preferences_v1")
    )
      return false;
    const source = preview.restore_source;
    if (!source?.snapshot_id || !Number.isFinite(Date.parse(source.expires_at)))
      return false;
    return store.state.switch_snapshots.some(
      (backup) =>
        backup.id === source.snapshot_id &&
        backup.account_id === preview.restore_owner.account_id &&
        backup.panel_origin === preview.restore_owner.panel_origin &&
        backup.installation_id === preview.restore_owner.installation_id &&
        backup.client === preview.client &&
        backup.expires_at === source.expires_at &&
        backup.file_count === preview.paths.length &&
        ["available", "conflict"].includes(backup.state),
    );
  }
  const connection = store.activeConnection;
  if (!connection) return false;
  return (
    connection.id === preview.context.connection_id &&
    connection.account_id === preview.context.account_id &&
    connection.revision === preview.context.connection_revision &&
    connection.credential_revision === preview.context.credential_revision &&
    connection.node_id === preview.context.node_id &&
    store.models.some(
      (model) =>
        model.id === preview.model_id &&
        model.revision === preview.context.model_revision,
    )
  );
});
watch(
  () => [props.preview?.id ?? null, current.value] as const,
  ([id, valid]) => {
    if (id && !valid) rejectedPreviewIds.value.add(id);
  },
  { immediate: true, flush: "sync" },
);
const allowed = computed(
  () =>
    current.value &&
    !store.state.maintenance_active &&
    props.preview?.can_apply &&
    codexReady.value &&
    (props.preview.operation === "restore" || store.ready) &&
    !configBusy.value,
);
const preservingClaudePreferences = computed(
  () =>
    props.preview?.operation === "restore" &&
    props.preview.client === "claude" &&
    props.preview.restore_coordination === "claude_main_preferences_v1",
);
const codexReady = computed(() => {
  const preview = props.preview;
  if (preview?.client !== "codex" || preview.operation !== "apply") return true;
  return (
    !!preview.codex_catalog && !!preview.codex_configuration_target?.cli_version
  );
});
const codexRuntimeVerified = computed(() => {
  const runtime = props.preview?.codex_runtime;
  return (
    runtime?.state === "verified" && !!runtime.revision && !!runtime.cli_version
  );
});
const configBusy = computed(
  () => store.busy.has("config-apply") || store.busy.has("config-export"),
);
const exportAllowed = computed(
  () =>
    current.value &&
    !store.state.maintenance_active &&
    props.preview?.operation === "apply" &&
    props.preview.client === "claude" &&
    props.preview.can_export_import &&
    store.ready &&
    !configBusy.value,
);
async function exportImport() {
  const preview = props.preview;
  if (!preview || !exportAllowed.value) return;
  const result = await store.perform("config-export", "export_claude_import", {
    preview_id: preview.id,
    confirmed: true,
  });
  if (!result) rejectedPreviewIds.value.add(preview.id);
  if (result?.saved) {
    store.notify(t("access.importExported"), "neutral");
    emit("close");
  }
}
async function confirm() {
  const preview = props.preview;
  if (!preview || !allowed.value) return;
  const preservingPreferences =
    preview.operation === "restore" &&
    preview.client === "claude" &&
    preview.restore_coordination === "claude_main_preferences_v1";
  const result =
    preview.operation === "restore"
      ? await store.perform("config-apply", "restore_client_config", {
          preview_id: preview.id,
          confirmed: true,
        })
      : await store.perform("config-apply", "apply_client_config", {
          preview_id: preview.id,
          confirmed: true,
        });
  if (!result) {
    rejectedPreviewIds.value.add(preview.id);
    if (preview.client === "claude") await store.refresh();
  }
  if (result) {
    await store.refresh();
    store.notify(
      result.state === "conflict"
        ? t("client.conflict")
        : t(
            result.state === "restored"
              ? preservingPreferences
                ? "access.restoredPreservingPreferences"
                : "access.restored"
              : "access.saved",
          ),
      result.state === "conflict" ? "error" : "neutral",
    );
    emit("applied");
  }
}
</script>
<template>
  <OrbitDialog
    :open="preview !== null"
    :title="t('access.previewTitle')"
    :description="t('access.previewHint')"
    wide
    @update:open="
      (value) => {
        if (!value && !configBusy) emit('close');
      }
    "
  >
    <template v-if="preview"
      ><dl class="preview-context">
        <div>
          <dt>{{ t("common.account") }}</dt>
          <dd>
            {{
              preview.operation === "apply"
                ? preview.context.account_label
                : preview.restore_owner.account_label
            }}
          </dd>
        </div>
        <div v-if="preview.operation === 'apply'">
          <dt>{{ t("access.node") }}</dt>
          <dd>{{ previewNodeName }}</dd>
        </div>
        <div>
          <dt>{{ t("common.version") }}</dt>
          <dd>
            {{ preview.client }} ·
            {{ preview.target_version ?? t("common.unknown") }}
          </dd>
        </div>
      </dl>
      <template v-if="preview.client === 'codex'">
        <h3>{{ t("access.codexConfigurationTarget") }}</h3>
        <p class="info-box">{{ t("access.codexDefaultScope") }}</p>
        <dl class="preview-context">
          <div>
            <dt>{{ t("access.codexConfigurationCli") }}</dt>
            <dd>
              {{
                preview.codex_configuration_target?.cli_version ??
                t("common.unknown")
              }}
            </dd>
          </div>
        </dl>
        <p
          v-if="targetClient?.managed === 'unknown'"
          class="info-box"
          role="status"
        >
          {{ t("access.remotePolicyWarning") }}
        </p>
        <template v-if="preview.operation === 'apply'">
          <h3>{{ t("access.codexCatalog") }}</h3>
          <template v-if="preview.codex_catalog">
            <dl class="preview-context">
              <div>
                <dt>{{ t("access.codexDefaultAlias") }}</dt>
                <dd>
                  <code>{{ preview.codex_catalog.default_alias }}</code>
                </dd>
              </div>
              <div>
                <dt>{{ t("access.codexDefaultEffort") }}</dt>
                <dd>{{ preview.codex_catalog.default_reasoning_effort }}</dd>
              </div>
              <div>
                <dt>{{ t("access.codexDeclaredCli") }}</dt>
                <dd>{{ preview.codex_catalog.cli_version }}</dd>
              </div>
            </dl>
            <h4>{{ t("access.codexMenuAliases") }}</h4>
            <ul class="path-list" :aria-label="t('access.codexMenuAliases')">
              <li v-for="alias in preview.codex_catalog.aliases" :key="alias">
                <code>{{ alias }}</code>
              </li>
            </ul>
          </template>
          <p v-else class="info-box">
            {{ t("access.codexCatalogUnavailable") }}
          </p>
        </template>
        <dl class="preview-context">
          <div>
            <dt>{{ t("access.codexRuntime") }}</dt>
            <dd>
              {{
                codexRuntimeVerified
                  ? t("access.codexRuntimeVerified")
                  : t("access.codexRuntimeUnverified")
              }}
            </dd>
          </div>
          <div>
            <dt>{{ t("access.codexRuntimeCli") }}</dt>
            <dd>
              {{ preview.codex_runtime?.cli_version ?? t("common.unknown") }}
            </dd>
          </div>
        </dl>
        <p v-if="!codexRuntimeVerified" class="info-box">
          {{ t("access.codexRuntimeBlocked") }}
        </p>
        <p class="info-box">{{ t("access.codexEvidenceBoundary") }}</p>
        <p v-if="!codexReady" class="error-text" role="alert">
          {{ t("access.codexConfigurationUnavailable") }}
        </p>
      </template>
      <template v-if="preview.client === 'claude'">
        <h3>{{ t("access.claudeConfigurationTarget") }}</h3>
        <p class="info-box">{{ t("access.claudeDesktopScope") }}</p>
        <dl class="preview-context">
          <div>
            <dt>{{ t("common.version") }}</dt>
            <dd>
              {{
                preview.claude_configuration_target?.app_version ??
                t("common.unknown")
              }}
            </dd>
          </div>
        </dl>
        <p
          v-if="targetClient?.managed === 'unknown'"
          class="info-box"
          role="status"
        >
          {{ t("access.remotePolicyWarning") }}
        </p>
      </template>
      <h3>{{ t("access.paths") }}</h3>
      <ul class="path-list">
        <li v-for="path in preview.paths" :key="path">
          <code>{{ path }}</code>
        </li>
      </ul>
      <h3>{{ t("access.impact") }}</h3>
      <ul class="impact-list">
        <li v-for="impact in preview.impact" :key="impact">
          {{ redactServiceUrls(impact) }}
        </li>
      </ul>
      <h3>{{ t("access.changes") }}</h3>
      <pre class="diff-preview">{{
        redactServiceUrls(preview.redacted_diff)
      }}</pre>
      <p v-if="!current" class="error-text" role="alert">
        {{
          t(
            preview.operation === "restore"
              ? "access.restorePreviewExpired"
              : "access.previewExpired",
          )
        }}
      </p>
      <p v-if="preview.blocking_reason" class="info-box">
        {{
          configPolicyMessage(preview.blocking_reason) ??
          preview.blocking_reason
        }}
      </p>
      <p v-if="preview.can_export_import" class="info-box">
        {{ t("access.importHint") }}
      </p>
      <div class="dialog-actions">
        <button
          class="button secondary"
          :disabled="configBusy"
          @click="emit('close')"
        >
          {{ t("action.cancel") }}</button
        ><button
          v-if="preview.can_export_import"
          class="button secondary"
          :disabled="!exportAllowed"
          @click="exportImport"
        >
          {{ t("access.exportImport") }}
        </button>
        <button class="button primary" :disabled="!allowed" @click="confirm">
          <OrbitIcon name="check" />{{
            t(
              preview.operation === "apply"
                ? "access.saveLocalConfiguration"
                : preservingClaudePreferences
                  ? "access.restorePreservingPreferences"
                  : "access.restoreLocalConfiguration",
            )
          }}
        </button>
      </div></template
    >
  </OrbitDialog>
</template>
