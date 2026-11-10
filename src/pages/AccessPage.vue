<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import { t } from "../shared/i18n";
import { nodeName } from "../shared/format";
import type {
  ClaudeRestoreCoordination,
  ClaudeRoleModels,
  ClientKind,
  ConfigPreview,
} from "../shared/contracts";
import { CAPABILITIES } from "../shared/contracts";
import ClientRow from "../components/ClientRow.vue";
import ConfigReviewDialog from "../components/ConfigReviewDialog.vue";
import OrbitDialog from "../components/OrbitDialog.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
import StatusBadge from "../components/StatusBadge.vue";
const store = useOrbitStore();
const claudeRoles = [
  { key: "primary", label: "access.rolePrimary" },
  { key: "fallback", label: "access.roleFallback" },
  { key: "auxiliary", label: "access.roleAuxiliary" },
] as const;
const selectedRoles = ref<ClaudeRoleModels>({
  primary: "",
  fallback: "",
  auxiliary: "",
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
const manageAction = ref<"revoke" | "delete" | null>(null);
const revokeOnDelete = ref(false);
const manageConnectionId = ref<string | null>(null);
const manageRevision = ref<number | null>(null);
const manageCredentialRevision = ref<string | null>(null);
const recoveryScope = ref("");
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
const historicalBackups = computed(() =>
  store.state.backups.filter((backup) => backup.kind !== "switch"),
);
function switchSnapshot(kind: ClientKind) {
  return store.state.backups.find(
    (backup) => backup.client === kind && backup.kind === "switch",
  );
}
const backupScopes = computed(() =>
  Array.from(
    new Map(
      historicalBackups.value.map((backup) => [
        `${backup.panel_origin}/${backup.account_id}`,
        {
          id: `${backup.panel_origin}/${backup.account_id}`,
          label: backup.account_label,
        },
      ]),
    ).values(),
  ),
);
const visibleBackups = computed(() =>
  historicalBackups.value.filter(
    (backup) =>
      `${backup.panel_origin}/${backup.account_id}` === recoveryScope.value,
  ),
);
const claudeModels = computed(() =>
  store.textModels.filter(
    (model) =>
      model.provider_id === store.activeConnection?.provider_id &&
      model.capabilities.includes(CAPABILITIES.messages),
  ),
);
const codexModelsAvailable = computed(() =>
  store.textModels.some(
    (model) =>
      model.provider_id === store.activeConnection?.provider_id &&
      model.capabilities.includes(CAPABILITIES.responses),
  ),
);
const rolesReady = computed(() =>
  claudeRoles.every(({ key }) =>
    claudeModels.value.some((model) => model.id === selectedRoles.value[key]),
  ),
);
const manageCurrent = computed(
  () =>
    !!manageConnectionId.value &&
    manageConnectionId.value === store.activeConnection?.id &&
    manageRevision.value === store.activeConnection.revision &&
    manageCredentialRevision.value ===
      store.activeConnection.credential_revision,
);
watch(
  () => store.scope,
  () => {
    previewGeneration++;
    selectedRoles.value = { primary: "", fallback: "", auxiliary: "" };
  },
  { flush: "sync" },
);
watch(
  () => store.scope,
  () => {
    recoveryScope.value = store.activeConnection
      ? `${store.activeConnection.panel_origin}/${store.activeConnection.account_id}`
      : "";
  },
  { immediate: true },
);
watch(claudeModels, (models) => {
  for (const { key } of claudeRoles) {
    if (!models.some((model) => model.id === selectedRoles.value[key]))
      selectedRoles.value[key] = "";
  }
});
async function requestPreview(client: ClientKind) {
  const connection = store.activeConnection;
  if (
    !connection ||
    switchSnapshot(client) ||
    (client === "claude" ? !rolesReady.value : !codexModelsAvailable.value) ||
    !previewAllowed(client) ||
    previewing.value[client]
  )
    return;
  const scope = store.scope;
  const origin = connection.panel_origin;
  const roles = client === "claude" ? { ...selectedRoles.value } : undefined;
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
        ...(roles ? { claude_models: roles } : {}),
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
      (roles &&
        (result.model_id !== roles.primary ||
          !rolesReady.value ||
          claudeRoles.some(
            ({ key }) => roles[key] !== selectedRoles.value[key],
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
  const backup = store.state.backups.find((item) => item.id === id);
  if (
    !backup ||
    (coordination && backup.client !== "claude") ||
    !configurationAllowed(backup.client) ||
    store.busy.has("preview-restore")
  )
    return;
  const expected = { ...backup };
  const generation = ++previewGeneration;
  const result = await store.perform("preview-restore", "preview_restore", {
    backup_group_id: id,
    ...(coordination ? { coordination } : {}),
  });
  if (!result || generation !== previewGeneration) return;
  if (
    result.operation !== "restore" ||
    (result.restore_coordination ?? undefined) !== coordination ||
    result.restore_source?.backup_group_id !== id ||
    result.restore_source.kind !== expected.kind ||
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
function manage(action: "revoke" | "delete") {
  const connection = store.activeConnection;
  manageConnectionId.value = connection?.id ?? null;
  manageRevision.value = connection?.revision ?? null;
  manageCredentialRevision.value = connection?.credential_revision ?? null;
  revokeOnDelete.value = false;
  manageAction.value = action;
}
async function confirmManage() {
  if (
    !manageAction.value ||
    !manageCurrent.value ||
    !manageConnectionId.value ||
    manageRevision.value === null ||
    ((manageAction.value === "revoke" || revokeOnDelete.value) &&
      !manageCredentialRevision.value)
  )
    return;
  const result =
    manageAction.value === "revoke"
      ? await store.perform("manage-connection", "revoke_inference_key", {
          connection_id: manageConnectionId.value,
          expected_credential_revision: manageCredentialRevision.value!,
          confirmed: true,
        })
      : await store.perform("manage-connection", "delete_connection", {
          connection_id: manageConnectionId.value,
          expected_revision: manageRevision.value,
          expected_credential_revision: manageCredentialRevision.value,
          revoke_inference_key: revokeOnDelete.value,
          confirmed: true,
        });
  if (result) {
    store.acceptState(result);
    manageAction.value = null;
  }
}
async function backupNow(client: ClientKind) {
  const result = await store.perform(
    `backup-${client}`,
    "backup_client_config",
    { client },
  );
  if (result) {
    store.acceptState(result);
    store.notify(t("access.backupSaved"), "success");
  }
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
      <label for="access-node" class="field-label">{{
        t("access.node")
      }}</label>
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
        :options="
          store.state.nodes.map((node, index) => ({
            value: node.id,
            label: node.available
              ? nodeName(node, index)
              : `${nodeName(node, index)} · ${t('access.nodeUnavailable')}`,
            disabled: !node.available,
          }))
        "
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
          <template v-else>
            <p class="muted">{{ t("access.claudeRolesHint") }}</p>
            <div class="claude-roles">
              <div v-for="role in claudeRoles" :key="role.key" class="field">
                <label :for="`claude-${role.key}`" class="field-label">{{
                  t(role.label)
                }}</label>
                <OrbitSelect
                  :id="`claude-${role.key}`"
                  v-model="selectedRoles[role.key]"
                  :options="
                    claudeModels.map((model) => ({
                      value: model.id,
                      label: model.label,
                    }))
                  "
                  :placeholder="
                    t(
                      store.modelsLoading
                        ? 'common.loading'
                        : 'access.selectModel',
                    )
                  "
                  :disabled="
                    store.modelsLoading ||
                    !claudeModels.length ||
                    previewing.claude
                  "
                />
              </div>
            </div>
          </template>
          <button
            class="button primary"
            :disabled="
              !store.ready ||
              !previewAllowed(client.client) ||
              (client.client === 'claude'
                ? !rolesReady
                : !codexModelsAvailable) ||
              store.modelsLoading ||
              previewing[client.client] ||
              store.busy.has(`preview-${client.client}`)
            "
            @click="requestPreview(client.client)"
          >
            <OrbitIcon name="access" />{{ t("access.connectOrbit") }}
          </button>
        </template>
      </div>
      <details
        v-if="client.manual_steps.length || client.config_paths.length"
        class="client-technical"
      >
        <summary>{{ t("access.technicalDetails") }}</summary>
        <p v-for="step in client.manual_steps" :key="step" class="manual-step">
          {{ step }}
        </p>
        <div v-if="client.config_paths.length" class="client-paths">
          <OrbitIcon name="folder" /><span>{{
            client.config_paths.join(" · ")
          }}</span>
        </div>
      </details>
    </article>
  </section>
  <section class="section-block">
    <div class="section-heading">
      <h2>{{ t("access.backups") }}</h2>
    </div>
    <p class="muted section-description">{{ t("access.backupHint") }}</p>
    <div class="manual-backup-actions">
      <button
        v-for="client in store.state.clients"
        :key="client.client"
        type="button"
        class="button secondary"
        :disabled="
          !store.state.active_connection_id ||
          !configurationAllowed(client.client) ||
          !!switchSnapshot(client.client) ||
          store.state.maintenance_active ||
          store.busy.has(`backup-${client.client}`)
        "
        @click="backupNow(client.client)"
      >
        <OrbitIcon name="file" />{{ t("access.backupNow") }} ·
        {{ t(client.client === "codex" ? "home.codex" : "home.claude") }}
      </button>
    </div>
    <div v-if="backupScopes.length" class="field backup-scope">
      <label for="backup-account" class="field-label">{{
        t("access.backupAccount")
      }}</label
      ><OrbitSelect
        id="backup-account"
        v-model="recoveryScope"
        :options="
          backupScopes.map((scope) => ({ value: scope.id, label: scope.label }))
        "
        :placeholder="t('access.selectBackupAccount')"
      />
    </div>
    <div v-if="!visibleBackups.length" class="empty-inline">
      <OrbitIcon name="history" />{{ t("access.noBackups") }}
    </div>
    <div v-for="backup in visibleBackups" :key="backup.id" class="backup-row">
      <OrbitIcon name="file" />
      <div>
        <strong>{{
          backup.client === "codex" ? t("home.codex") : t("home.claude")
        }}</strong>
        <p class="muted">{{ new Date(backup.created_at).toLocaleString() }}</p>
      </div>
      <StatusBadge
        v-if="backup.state !== 'available'"
        :text="
          t(
            backup.state === 'conflict'
              ? 'client.conflict'
              : 'common.unavailable',
          )
        "
        tone="warning"
      /><button
        class="button secondary"
        :disabled="
          backup.state === 'unavailable' ||
          !!switchSnapshot(backup.client) ||
          !configurationAllowed(backup.client) ||
          store.busy.has('preview-restore')
        "
        @click="restore(backup.id)"
      >
        {{ t("action.restore") }}
      </button>
      <button
        v-if="
          backup.client === 'claude' &&
          !backup.kind &&
          backup.state !== 'unavailable'
        "
        class="button secondary"
        :disabled="
          !!switchSnapshot(backup.client) ||
          !configurationAllowed(backup.client) ||
          store.busy.has('preview-restore')
        "
        :title="t('access.preferencesRestoreHint')"
        @click="restore(backup.id, 'claude_main_preferences_v1')"
      >
        {{ t("access.preserveClaudePreferences") }}
      </button>
    </div>
  </section>
  <details v-if="store.activeConnection" class="connection-actions">
    <summary>{{ t("action.more") }}</summary>
    <button
      class="text-button danger"
      :disabled="!store.activeConnection.credential_revision"
      @click="manage('revoke')"
    >
      {{ t("access.revoke") }}</button
    ><button class="text-button" @click="manage('delete')">
      {{ t("access.remove") }}
    </button>
  </details>
  <ConfigReviewDialog
    :preview="preview"
    @close="preview = null"
    @applied="preview = null"
  />
  <OrbitDialog
    :open="manageAction !== null"
    :title="t(manageAction === 'revoke' ? 'access.revoke' : 'access.remove')"
    :description="
      t(manageAction === 'revoke' ? 'access.revokeHint' : 'access.removeHint')
    "
    @update:open="
      (value) => {
        if (!value) manageAction = null;
      }
    "
    ><label v-if="manageAction === 'delete'" class="checkbox-row"
      ><input v-model="revokeOnDelete" type="checkbox" />{{
        t("access.revoke")
      }}</label
    >
    <p v-if="!manageCurrent" class="error-text">
      {{ t("access.previewExpired") }}
    </p>
    <div class="dialog-actions">
      <button class="button secondary" @click="manageAction = null">
        {{ t("action.cancel") }}</button
      ><button
        class="button danger-button"
        :disabled="
          !manageCurrent ||
          ((manageAction === 'revoke' || revokeOnDelete) &&
            !manageCredentialRevision) ||
          store.busy.has('manage-connection')
        "
        @click="confirmManage"
      >
        {{ t("common.confirm") }}
      </button>
    </div></OrbitDialog
  >
</template>
