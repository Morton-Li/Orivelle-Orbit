<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import { t } from "../shared/i18n";
import type { ClientKind, ConfigPreview } from "../shared/contracts";
import { CAPABILITIES } from "../shared/contracts";
import ClientRow from "../components/ClientRow.vue";
import ConfigReviewDialog from "../components/ConfigReviewDialog.vue";
import OrbitDialog from "../components/OrbitDialog.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
import StatusBadge from "../components/StatusBadge.vue";
const store = useOrbitStore();
const selected = ref<Record<ClientKind, string>>({ codex: "", claude: "" });
const preview = ref<ConfigPreview | null>(null);
const manageAction = ref<"revoke" | "delete" | null>(null);
const revokeOnDelete = ref(false);
const manageConnectionId = ref<string | null>(null);
const manageRevision = ref<number | null>(null);
const manageCredentialRevision = ref<string | null>(null);
const recoveryScope = ref("");
const codexConfigurationAllowed = computed(() => {
  const client = store.state.clients.find((item) => item.client === "codex");
  return (
    client?.installed &&
    client.platform_supported &&
    client.managed !== "yes" &&
    client.configuration_access === "allowed" &&
    !!client.configuration_access_revision
  );
});
const backupScopes = computed(() =>
  Array.from(
    new Map(
      store.state.backups.map((backup) => [
        `${backup.panel_origin}/${backup.account_id}`,
        {
          id: `${backup.panel_origin}/${backup.account_id}`,
          label: `${backup.account_label} · ${backup.panel_origin}`,
        },
      ]),
    ).values(),
  ),
);
const visibleBackups = computed(() =>
  store.state.backups.filter(
    (backup) =>
      `${backup.panel_origin}/${backup.account_id}` === recoveryScope.value,
  ),
);
const modelsFor = (client: ClientKind) =>
  store.textModels.filter((model) =>
    model.capabilities.includes(
      client === "codex" ? CAPABILITIES.responses : CAPABILITIES.messages,
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
    selected.value = { codex: "", claude: "" };
  },
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
watch(
  () => store.textModels,
  () => {
    for (const kind of ["codex", "claude"] as const)
      if (!modelsFor(kind).some((model) => model.id === selected.value[kind]))
        selected.value[kind] = "";
  },
);
async function requestPreview(client: ClientKind) {
  const connection = store.activeConnection;
  if (
    !connection ||
    !selected.value[client] ||
    (client === "codex" && !codexConfigurationAllowed.value)
  )
    return;
  const result = await store.perform(
    `preview-${client}`,
    "preview_client_config",
    {
      connection_id: connection.id,
      expected_revision: connection.revision,
      client,
      model_id: selected.value[client],
    },
  );
  if (result) preview.value = result;
}
async function restore(id: string) {
  const backup = store.state.backups.find((item) => item.id === id);
  if (backup?.client === "codex" && !codexConfigurationAllowed.value) return;
  const result = await store.perform("preview-restore", "preview_restore", {
    backup_group_id: id,
  });
  if (result) preview.value = result;
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
function changeConnection(event: Event) {
  void store.selectConnection((event.target as HTMLSelectElement).value);
}
function changeNode(event: Event) {
  void store.selectNode((event.target as HTMLSelectElement).value);
}
</script>
<template>
  <section class="page-heading">
    <p class="eyebrow">{{ t("access.eyebrow") }}</p>
    <h1>{{ t("access.title") }}</h1>
    <p class="page-description">{{ t("access.description") }}</p>
  </section>
  <section class="connection-settings">
    <div class="field">
      <label for="access-connection" class="field-label">{{
        t("access.connection")
      }}</label>
      <div class="input-with-action">
        <select
          id="access-connection"
          :value="store.activeConnection?.id ?? ''"
          :disabled="!store.native || store.busy.has('connection')"
          @change="changeConnection"
        >
          <option value="" disabled>{{ t("access.noConnection") }}</option>
          <option
            v-for="connection in store.state.connections"
            :key="connection.id"
            :value="connection.id"
          >
            {{ connection.label }}
          </option></select
        ><button
          class="icon-button"
          :aria-label="t('action.login')"
          @click="store.loginOpen = true"
        >
          <OrbitIcon name="plus" />
        </button>
      </div>
      <p v-if="store.activeConnection" class="field-hint truncate">
        {{ store.activeConnection.panel_origin }}
      </p>
    </div>
    <div class="field">
      <label for="access-node" class="field-label">{{ t("access.node") }}</label
      ><select
        id="access-node"
        :value="store.activeConnection?.node_id ?? ''"
        :disabled="!store.activeConnection || store.busy.has('node')"
        @change="changeNode"
      >
        <option value="" disabled>{{ t("access.noNode") }}</option>
        <option
          v-for="node in store.state.nodes"
          :key="node.id"
          :value="node.id"
          :disabled="!node.available"
        >
          {{ node.label
          }}{{ node.available ? "" : ` · ${t("access.nodeUnavailable")}` }}
        </option>
      </select>
      <p
        v-if="store.activeConnection?.edge_base_url"
        class="field-hint truncate"
      >
        {{ store.activeConnection.edge_base_url }}
      </p>
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
      :key="client.client"
      class="access-client"
    >
      <ClientRow
        :client="client"
        :connection="store.activeConnection"
        :detected="store.initialized"
        compact
        ><span
      /></ClientRow>
      <div class="access-client-controls">
        <div class="field">
          <label :for="`model-${client.client}`" class="field-label">{{
            t("access.model")
          }}</label
          ><select
            :id="`model-${client.client}`"
            v-model="selected[client.client]"
            :disabled="!store.activeConnection || store.modelsLoading"
          >
            <option value="" disabled>
              {{
                t(store.modelsLoading ? "common.loading" : "access.selectModel")
              }}
            </option>
            <option
              v-for="model in modelsFor(client.client)"
              :key="model.id"
              :value="model.id"
            >
              {{ model.label
              }}{{
                model.support_status === "verified"
                  ? ""
                  : ` · ${t("common.pending")}`
              }}
            </option>
          </select>
        </div>
        <button
          class="button primary"
          :disabled="
            !store.ready ||
            !client.installed ||
            !selected[client.client] ||
            client.managed === 'yes' ||
            (client.client === 'codex' && !codexConfigurationAllowed) ||
            store.busy.has(`preview-${client.client}`)
          "
          @click="requestPreview(client.client)"
        >
          <OrbitIcon name="file" />{{ t("action.preview") }}
        </button>
      </div>
      <p v-for="step in client.manual_steps" :key="step" class="manual-step">
        {{ step }}
      </p>
      <div v-if="client.config_paths.length" class="client-paths">
        <OrbitIcon name="folder" /><span>{{
          client.config_paths.join(" · ")
        }}</span>
      </div>
    </article>
  </section>
  <section class="section-block">
    <div class="section-heading">
      <h2>{{ t("access.backups") }}</h2>
      <StatusBadge text="30 d · 5" />
    </div>
    <p class="muted section-description">{{ t("access.backupHint") }}</p>
    <div v-if="backupScopes.length" class="field backup-scope">
      <label for="backup-account" class="field-label">{{
        t("access.backupAccount")
      }}</label
      ><select id="backup-account" v-model="recoveryScope">
        <option value="" disabled>{{ t("access.selectBackupAccount") }}</option>
        <option
          v-for="choice in backupScopes"
          :key="choice.id"
          :value="choice.id"
        >
          {{ choice.label }}
        </option>
      </select>
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
          (backup.client === 'codex' && !codexConfigurationAllowed) ||
          store.busy.has('preview-restore')
        "
        @click="restore(backup.id)"
      >
        {{ t("action.restore") }}
      </button>
    </div>
  </section>
  <section v-if="store.activeConnection" class="connection-actions">
    <button
      class="text-button danger"
      :disabled="!store.activeConnection.credential_revision"
      @click="manage('revoke')"
    >
      {{ t("access.revoke") }}</button
    ><button class="text-button" @click="manage('delete')">
      {{ t("access.remove") }}
    </button>
  </section>
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
