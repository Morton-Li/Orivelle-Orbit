<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import type { ConfigPreview, ConfigResult } from "../shared/contracts";
import { t, type MessageKey } from "../shared/i18n";
import {
  matchesReviewConnection,
  matchesReviewModel,
} from "../shared/config-review";
import { configPolicyMessage, nodeName } from "../shared/format";
import OrbitDialog from "./OrbitDialog.vue";
import OrbitIcon from "./OrbitIcon.vue";
const props = defineProps<{
  preview: ConfigPreview | null;
  codexReviewOpen?: boolean;
  checking?: boolean;
  error?: string;
}>();
const emit = defineEmits<{
  close: [];
  retry: [];
  applied: [result: ConfigResult];
}>();
const store = useOrbitStore();
const now = ref(Date.now());
const failure = ref<{ id: string; message: string } | null>(null);
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
watch(
  () => props.preview?.expires_at,
  (expiry, _, onCleanup) => {
    if (!expiry) return;
    now.value = Date.now();
    const timer = window.setInterval(() => {
      now.value = Date.now();
    }, 1000);
    onCleanup(() => window.clearInterval(timer));
  },
  { immediate: true },
);

const codexReview = computed(() => {
  const preview = props.preview;
  return preview?.client === "codex" &&
    preview.operation === "apply" &&
    preview.expires_at === null
    ? preview
    : null;
});
const codexIssue = computed<MessageKey | null>(() => {
  const preview = codexReview.value;
  if (!preview) return null;
  if (!matchesReviewConnection(preview, store.activeConnection))
    return "access.scopeChanged";
  if (store.activeConnection?.permission_state === "readonly")
    return "access.readonly";
  if (["expired", "revoked", "logged_out"].includes(store.state.session.status))
    return "access.connectAccountFirst";
  if (
    ["revoked", "permission_denied"].includes(
      store.state.session.inference_credential_status,
    )
  )
    return "access.chooseKeyFirst";
  const client = targetClient.value;
  if (client?.managed === "yes" || client?.configuration_access === "blocked")
    return "client.configurationBlocked";
  if (client?.configuration_access === "unverified")
    return "access.checkConfiguration";
  if (!client?.installed) return "access.installClientFirst";
  if (!client.platform_supported || client.version !== preview.target_version)
    return "access.codexConfigurationChanged";
  if (
    !preview.codex_catalog ||
    !preview.codex_configuration_target?.cli_version ||
    !preview.codex_configuration_target.revision
  )
    return "access.codexConfigurationUnavailable";
  if (
    client.configuration_access !== "allowed" ||
    !client.configuration_access_revision
  )
    return "access.checkConfiguration";
  if (
    client.configuration_access_revision !==
    preview.codex_configuration_target.revision
  )
    return "access.codexConfigurationChanged";
  if (!matchesReviewModel(preview, store.models))
    return store.modelsLoading
      ? "access.waitModels"
      : "access.reviewModelsChanged";
  if (!store.ready) return "access.waitConnection";
  return null;
});
const current = computed(() => {
  const preview = props.preview;
  if (!preview || rejectedPreviewIds.value.has(preview.id)) return false;
  if (codexReview.value) return codexIssue.value === null;
  const client = targetClient.value;
  const expiresAt = preview.expires_at ? Date.parse(preview.expires_at) : NaN;
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
        !["claude_main_preferences_v1", "claude_main_settings_v2"].includes(
          preview.restore_coordination,
        ))
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
  return (
    matchesReviewConnection(preview, store.activeConnection) &&
    matchesReviewModel(preview, store.models)
  );
});
watch(
  () => [props.preview?.id, current.value, codexIssue.value] as const,
  ([id, valid, issue]) => {
    if (!id || valid || rejectedPreviewIds.value.has(id)) return;
    if (codexReview.value) {
      if (
        !issue ||
        issue === "access.checkConfiguration" ||
        issue === "access.waitConnection" ||
        issue === "access.waitModels"
      )
        return;
      failure.value = { id, message: t(issue) };
    }
    rejectedPreviewIds.value.add(id);
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
const codexReady = computed(() => {
  const preview = props.preview;
  if (preview?.client !== "codex" || preview.operation !== "apply") return true;
  return (
    !!preview.codex_catalog && !!preview.codex_configuration_target?.cli_version
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
  let message: string | undefined;
  const result = await store.perform(
    "config-apply",
    preview.operation === "restore"
      ? "restore_client_config"
      : "apply_client_config",
    { preview_id: preview.id, confirmed: true },
    {
      onError: (error) => {
        message = store.report(error);
      },
    },
  );
  if (!result) {
    if (preview.client === "codex")
      failure.value = {
        id: preview.id,
        message:
          message ??
          t(
            store.state.maintenance_active
              ? "common.updateMaintenance"
              : "common.failed",
          ),
      };
    rejectedPreviewIds.value.add(preview.id);
    if (preview.client === "claude") await store.refresh();
  }
  if (result) {
    await store.refresh();
    store.notify(
      t(
        result.state === "conflict"
          ? "client.conflict"
          : result.state === "restored"
            ? "access.disconnectedNotice"
            : "access.connectedNotice",
      ),
      result.state === "conflict" ? "error" : "success",
    );
    emit("applied", result);
  }
}
</script>
<template>
  <OrbitDialog
    :open="!!codexReviewOpen || preview !== null"
    :title="
      t(
        preview?.operation === 'restore'
          ? 'access.confirmDisconnect'
          : 'access.confirmConnect',
      )
    "
    :description="
      t(
        preview?.operation === 'restore'
          ? 'access.confirmDisconnectHint'
          : 'access.confirmConnectHint',
      )
    "
    @update:open="
      (value) => {
        if (!value && !configBusy) emit('close');
      }
    "
  >
    <template v-if="codexReviewOpen && (!preview || error)">
      <p v-if="error" class="error-text" role="alert">{{ error }}</p>
      <p v-else role="status">{{ t("access.checkingCodex") }}</p>
      <div class="dialog-actions">
        <button class="button secondary" @click="emit('close')">
          {{ t("action.cancel") }}
        </button>
        <button
          v-if="error"
          class="button primary"
          :disabled="checking"
          @click="emit('retry')"
        >
          {{ t("access.checkAgain") }}
        </button>
        <button v-else class="button primary" disabled>
          {{ t("access.confirmApply") }}
        </button>
      </div>
    </template>
    <template v-else-if="preview">
      <dl v-if="preview.operation === 'apply'" class="preview-context">
        <div>
          <dt>{{ t("access.node") }}</dt>
          <dd>{{ previewNodeName }}</dd>
        </div>
        <div v-if="preview.context.api_key">
          <dt>API Key</dt>
          <dd>
            {{ preview.context.api_key.name
            }}<small class="mono">{{ preview.context.api_key.prefix }}…</small>
          </dd>
        </div>
      </dl>
      <p
        v-if="!current && !preview.blocking_reason"
        class="error-text"
        role="alert"
      >
        {{
          failure?.id === preview.id
            ? failure.message
            : codexIssue
              ? t(codexIssue)
              : t(
                  preview.operation === "restore"
                    ? "access.restorePreviewExpired"
                    : "access.previewExpired",
                )
        }}
      </p>
      <p v-else-if="preview.blocking_reason" class="error-text" role="alert">
        {{
          configPolicyMessage(preview.blocking_reason) ??
          t("client.configurationBlocked")
        }}
      </p>
      <div class="dialog-actions">
        <button
          class="button secondary"
          :disabled="configBusy"
          @click="emit('close')"
        >
          {{ t("action.cancel") }}
        </button>
        <button
          v-if="preview.can_export_import"
          class="button secondary"
          :disabled="!exportAllowed"
          @click="exportImport"
        >
          {{ t("access.exportImport") }}
        </button>
        <button
          v-if="
            codexReviewOpen &&
            (!current || preview.blocking_reason) &&
            !configBusy
          "
          class="button secondary"
          @click="emit('retry')"
        >
          {{ t("access.checkAgain") }}
        </button>
        <button class="button primary" :disabled="!allowed" @click="confirm">
          <OrbitIcon name="check" />{{
            t(
              preview.operation === "apply"
                ? "access.confirmApply"
                : "access.confirmRestore",
            )
          }}
        </button>
      </div>
    </template>
  </OrbitDialog>
</template>
