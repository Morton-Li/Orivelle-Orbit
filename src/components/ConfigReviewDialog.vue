<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { useOrbitStore } from "../stores/orbit";
import type { ConfigPreview } from "../shared/contracts";
import { t } from "../shared/i18n";
import OrbitDialog from "./OrbitDialog.vue";
import OrbitIcon from "./OrbitIcon.vue";
const props = defineProps<{ preview: ConfigPreview | null }>();
const emit = defineEmits<{ close: []; applied: [] }>();
const store = useOrbitStore();
const now = ref(Date.now());
const timer = window.setInterval(() => {
  now.value = Date.now();
}, 1000);
onUnmounted(() => window.clearInterval(timer));
const current = computed(() => {
  const preview = props.preview;
  if (!preview) return false;
  const client = store.state.clients.find(
    (item) => item.client === preview.client,
  );
  if (
    client?.version !== preview.target_version ||
    Date.parse(preview.expires_at) <= now.value
  )
    return false;
  if (preview.operation === "restore")
    return store.state.backups.some(
      (backup) =>
        backup.account_id === preview.restore_owner.account_id &&
        backup.panel_origin === preview.restore_owner.panel_origin &&
        backup.installation_id === preview.restore_owner.installation_id &&
        backup.client === preview.client &&
        backup.state === "available",
    );
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
const allowed = computed(
  () =>
    current.value &&
    props.preview?.can_apply &&
    (props.preview.operation === "restore" || store.ready) &&
    !store.busy.has("config-apply"),
);
async function confirm() {
  const preview = props.preview;
  if (!preview || !allowed.value) return;
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
  if (result) {
    await store.refresh();
    store.notify(
      result.state === "conflict" ? t("client.conflict") : t("access.saved"),
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
        if (!value && !store.busy.has('config-apply')) emit('close');
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
          <dd>{{ preview.context.node_label }}</dd>
        </div>
        <div v-if="preview.operation === 'apply'">
          <dt>{{ t("access.model") }}</dt>
          <dd>{{ preview.model_id }}</dd>
        </div>
        <div v-else>
          <dt>{{ t("login.panel") }}</dt>
          <dd>{{ preview.restore_owner.panel_origin }}</dd>
        </div>
        <div>
          <dt>{{ t("common.version") }}</dt>
          <dd>
            {{ preview.client }} ·
            {{ preview.target_version ?? t("common.unknown") }}
          </dd>
        </div>
      </dl>
      <h3>{{ t("access.paths") }}</h3>
      <ul class="path-list">
        <li v-for="path in preview.paths" :key="path">
          <code>{{ path }}</code>
        </li>
      </ul>
      <h3>{{ t("access.impact") }}</h3>
      <ul class="impact-list">
        <li v-for="impact in preview.impact" :key="impact">{{ impact }}</li>
      </ul>
      <h3>{{ t("access.changes") }}</h3>
      <pre class="diff-preview">{{ preview.redacted_diff }}</pre>
      <p v-if="!current" class="error-text" role="alert">
        {{ t("access.previewExpired") }}
      </p>
      <p v-if="preview.blocking_reason" class="info-box">
        {{ preview.blocking_reason }}
      </p>
      <div class="dialog-actions">
        <button
          class="button secondary"
          :disabled="store.busy.has('config-apply')"
          @click="emit('close')"
        >
          {{ t("action.cancel") }}</button
        ><button class="button primary" :disabled="!allowed" @click="confirm">
          <OrbitIcon name="check" />{{ t("action.apply") }}
        </button>
      </div></template
    >
  </OrbitDialog>
</template>
