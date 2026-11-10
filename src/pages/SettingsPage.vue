<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useOrbitStore } from "../stores/orbit";
import type {
  DiagnosticsPreview,
  ForgetInstallationPreview,
  Locale,
  SettingsView,
  Theme,
  UpdateView,
} from "../shared/contracts";
import { formatBytes, redactServiceUrls } from "../shared/format";
import { t } from "../shared/i18n";
import OrbitDialog from "../components/OrbitDialog.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import StatusBadge from "../components/StatusBadge.vue";
const store = useOrbitStore();
const diagnostics = ref<DiagnosticsPreview | null>(null);
const forget = ref<ForgetInstallationPreview | null>(null);
const acknowledgeLoss = ref(false);
const logoutConnectionId = ref<string | null>(null);
const update = ref<UpdateView | null>(null);
const installOpen = ref(false);
const updateBusy = computed(() => store.busy.has("update"));
const settingsDisabled = computed(
  () =>
    !store.native ||
    store.state.maintenance_active ||
    store.busy.has("settings"),
);
const installationId = ref("");
watch(
  () => [store.scope, store.state.installations],
  () => {
    if (
      !store.state.installations.some(
        (item) => item.id === installationId.value,
      )
    )
      installationId.value =
        store.state.installations.find(
          (item) => item.panel_origin === store.activeConnection?.panel_origin,
        )?.id ??
        (store.state.installations.length === 1
          ? store.state.installations[0]!.id
          : "");
  },
  { immediate: true },
);
async function setting<K extends keyof SettingsView>(
  key: K,
  value: SettingsView[K],
) {
  await store.saveSettings({ [key]: value });
}
async function logout() {
  if (
    !logoutConnectionId.value ||
    logoutConnectionId.value !== store.activeConnection?.id
  )
    return;
  const result = await store.perform("logout", "logout", {
    connection_id: logoutConnectionId.value,
  });
  if (result) {
    store.acceptState(result);
    logoutConnectionId.value = null;
  }
}
async function previewForget() {
  if (!installationId.value) return;
  const result = await store.perform(
    "forget-preview",
    "preview_forget_installation",
    { installation_id: installationId.value },
  );
  if (result) {
    acknowledgeLoss.value = false;
    forget.value = result;
  }
}
async function forgetInstallation() {
  if (!forget.value || !acknowledgeLoss.value) return;
  const result = await store.perform("forget", "forget_installation", {
    preview_id: forget.value.id,
    acknowledged_backup_loss: true,
    confirmed: true,
  });
  if (result) {
    store.acceptState(result);
    forget.value = null;
  }
}
async function prepareDiagnostics() {
  const result = await store.perform("diagnostics", "prepare_diagnostics", {});
  if (result) diagnostics.value = result;
}
async function exportDiagnostics() {
  if (!diagnostics.value) return;
  const result = await store.perform(
    "diagnostics-export",
    "export_diagnostics",
    { preview_id: diagnostics.value.id },
  );
  if (result?.saved) store.notify(t("common.saved"), "success");
}
async function checkUpdate() {
  const result = await store.perform("update", "check_update", {});
  if (result) update.value = result;
}
async function downloadUpdate() {
  if (!update.value?.version || update.value.status !== "available") return;
  const result = await store.perform("update", "download_update", {
    version: update.value.version,
    confirmed: true,
  });
  if (result) update.value = result;
}
async function installUpdate() {
  if (!update.value?.version || update.value.status !== "verified") return;
  const result = await store.perform("update", "install_update", {
    version: update.value.version,
    confirmed: true,
  });
  if (result) {
    update.value = result;
    installOpen.value = false;
  }
}
</script>
<template>
  <section class="page-heading">
    <p class="eyebrow">{{ t("settings.eyebrow") }}</p>
    <h1>{{ t("settings.title") }}</h1>
    <p class="page-description">{{ t("settings.description") }}</p>
  </section>
  <div class="settings-sections">
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="sun" />
        <h2>{{ t("settings.appearance") }}</h2>
      </div>
      <div class="settings-row">
        <label for="setting-theme">{{ t("settings.theme") }}</label
        ><OrbitSelect
          id="setting-theme"
          :model-value="store.state.settings.theme"
          :disabled="settingsDisabled"
          :options="[
            { value: 'system', label: t('settings.system') },
            { value: 'light', label: t('settings.light') },
            { value: 'dark', label: t('settings.dark') },
          ]"
          @update:model-value="setting('theme', $event as Theme)"
        />
      </div>
      <div class="settings-row">
        <label for="setting-locale">{{ t("settings.language") }}</label
        ><OrbitSelect
          id="setting-locale"
          :model-value="store.state.settings.locale"
          :disabled="settingsDisabled"
          :options="[
            { value: 'zh-CN', label: '简体中文' },
            { value: 'en', label: 'English' },
          ]"
          @update:model-value="setting('locale', $event as Locale)"
        />
      </div>
    </section>
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="folder" />
        <h2>{{ t("settings.files") }}</h2>
      </div>
      <div class="settings-row">
        <label for="setting-disk">{{ t("settings.disk") }}</label
        ><OrbitSelect
          id="setting-disk"
          :model-value="String(store.state.settings.image_disk_limit_bytes)"
          :disabled="settingsDisabled"
          :options="
            [1, 2, 5, 10].map((gib) => ({
              value: String(gib * 1024 ** 3),
              label: `${gib} GiB`,
            }))
          "
          @update:model-value="
            setting('image_disk_limit_bytes', Number($event))
          "
        />
      </div>
      <div class="settings-row">
        <label for="setting-cache">{{ t("settings.cache") }}</label
        ><OrbitSelect
          id="setting-cache"
          :model-value="String(store.state.settings.preview_cache_limit_bytes)"
          :disabled="settingsDisabled"
          :options="
            [64, 128, 256].map((mib) => ({
              value: String(mib * 1024 ** 2),
              label: `${mib} MiB`,
            }))
          "
          @update:model-value="
            setting('preview_cache_limit_bytes', Number($event))
          "
        />
      </div>
      <p class="field-hint">{{ t("settings.diskHint") }}</p>
    </section>
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="shield" />
        <h2>{{ t("settings.security") }}</h2>
        <StatusBadge
          v-if="store.state.session.status === 'vault_unavailable'"
          :text="t('common.unavailable')"
          tone="error"
        />
      </div>
      <p class="muted">{{ t("settings.securityHint") }}</p>
      <p class="field-hint">{{ t("settings.logoutHint") }}</p>
      <div v-if="store.state.installations.length > 1" class="settings-row">
        <label for="forget-installation">{{
          t("settings.installation")
        }}</label>
        <OrbitSelect
          id="forget-installation"
          v-model="installationId"
          :placeholder="t('settings.installation')"
          :options="
            store.state.installations.map((installation, index) => ({
              value: installation.id,
              label:
                installation.account_labels.join(' · ') ||
                `${t('settings.installation')} ${index + 1}`,
            }))
          "
        />
      </div>
      <div class="settings-actions">
        <button
          class="button secondary"
          :disabled="
            !store.activeConnection ||
            !['authenticated', 'refreshing'].includes(
              store.state.session.status,
            )
          "
          @click="logoutConnectionId = store.activeConnection?.id ?? null"
        >
          {{ t("action.logout") }}</button
        ><button
          class="text-button danger"
          :disabled="
            !store.native || !installationId || store.busy.has('forget-preview')
          "
          @click="previewForget"
        >
          {{ t("settings.forget") }}
        </button>
      </div>
    </section>
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="window" />
        <h2>{{ t("settings.window") }}</h2>
      </div>
      <div class="settings-row">
        <label for="setting-close">{{ t("settings.close") }}</label
        ><OrbitSelect
          id="setting-close"
          :model-value="store.state.settings.close_behavior"
          :disabled="settingsDisabled"
          :options="[
            { value: 'platform_default', label: t('settings.platformDefault') },
            ...(store.state.platform === 'macos'
              ? [{ value: 'keep_running', label: t('settings.keepRunning') }]
              : []),
            { value: 'quit', label: t('settings.quit') },
          ]"
          @update:model-value="
            setting('close_behavior', $event as SettingsView['close_behavior'])
          "
        />
      </div>
      <p class="field-hint">{{ t("settings.quitHint") }}</p>
    </section>
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="download" />
        <h2>{{ t("settings.updates") }}</h2>
        <span v-if="update" class="version-label">{{
          update.current_version
        }}</span>
      </div>
      <label class="checkbox-row"
        ><input
          type="checkbox"
          :checked="store.state.settings.check_updates_on_start"
          :disabled="settingsDisabled"
          @change="
            setting(
              'check_updates_on_start',
              ($event.target as HTMLInputElement).checked,
            )
          "
        />{{ t("settings.autoCheck") }}</label
      >
      <p class="muted">
        {{
          update?.status === "up_to_date"
            ? t("settings.upToDate")
            : update?.status === "available"
              ? `${t("settings.updateAvailable")} · ${update.version}`
              : update?.status === "verified"
                ? `${t("settings.updateVerified")} · ${update.version}`
                : (update?.reason ?? t("settings.disabledUpdate"))
        }}
      </p>
      <div class="settings-actions">
        <button
          class="button secondary"
          :disabled="!store.native || updateBusy"
          @click="checkUpdate"
        >
          {{ t("settings.check") }}</button
        ><button
          v-if="update?.status === 'available'"
          class="button primary"
          :disabled="updateBusy"
          @click="downloadUpdate"
        >
          {{ t("settings.download") }}</button
        ><button
          v-if="update?.status === 'verified'"
          class="button primary"
          :disabled="updateBusy"
          @click="installOpen = true"
        >
          {{ t("settings.install") }}
        </button>
      </div>
    </section>
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="file" />
        <h2>{{ t("settings.diagnostics") }}</h2>
      </div>
      <p class="muted">{{ t("settings.diagnosticsHint") }}</p>
      <button
        class="button secondary"
        :disabled="!store.native || store.busy.has('diagnostics')"
        @click="prepareDiagnostics"
      >
        {{ t("settings.prepareDiagnostics") }}
      </button>
    </section>
  </div>
  <OrbitDialog
    :open="diagnostics !== null"
    :title="t('settings.prepareDiagnostics')"
    :description="t('settings.diagnosticsHint')"
    wide
    @update:open="
      (value) => {
        if (!value) diagnostics = null;
      }
    "
  >
    <pre class="diagnostics-preview">{{
      redactServiceUrls(diagnostics?.text ?? "")
    }}</pre>
    <div class="dialog-actions">
      <button class="button secondary" @click="diagnostics = null">
        {{ t("action.close") }}</button
      ><button
        class="button primary"
        :disabled="store.busy.has('diagnostics-export')"
        @click="exportDiagnostics"
      >
        {{ t("settings.exportDiagnostics") }}
      </button>
    </div></OrbitDialog
  >
  <OrbitDialog
    :open="forget !== null"
    :title="t('settings.forgetTitle')"
    :description="t('settings.forgetHint')"
    @update:open="
      (value) => {
        if (!value) forget = null;
      }
    "
    ><template v-if="forget">
      <p>
        {{ forget.affected_account_labels.join(" · ") }} ·
        {{ forget.backup_group_count }} {{ t("access.backups") }}
      </p>
      <ul class="impact-list">
        <li v-for="consequence in forget.consequences" :key="consequence">
          {{ consequence }}
        </li>
      </ul>
      <label class="checkbox-row"
        ><input v-model="acknowledgeLoss" type="checkbox" />{{
          t("settings.ackLoss")
        }}</label
      >
      <div class="dialog-actions">
        <button class="button secondary" @click="forget = null">
          {{ t("action.cancel") }}</button
        ><button
          class="button danger-button"
          :disabled="!acknowledgeLoss || store.busy.has('forget')"
          @click="forgetInstallation"
        >
          {{ t("common.confirm") }}
        </button>
      </div></template
    ></OrbitDialog
  >
  <OrbitDialog
    :open="logoutConnectionId !== null"
    :title="t('action.logout')"
    :description="t('settings.logoutHint')"
    @update:open="
      (value) => {
        if (!value) logoutConnectionId = null;
      }
    "
    ><div class="dialog-actions">
      <button class="button secondary" @click="logoutConnectionId = null">
        {{ t("action.cancel") }}</button
      ><button
        class="button primary"
        :disabled="
          logoutConnectionId !== store.activeConnection?.id ||
          store.busy.has('logout')
        "
        @click="logout"
      >
        {{ t("common.confirm") }}
      </button>
    </div></OrbitDialog
  >
  <OrbitDialog
    v-model:open="installOpen"
    :title="t('settings.install')"
    :description="t('settings.installHint')"
    ><div class="dialog-actions">
      <button class="button secondary" @click="installOpen = false">
        {{ t("action.cancel") }}</button
      ><button
        class="button primary"
        :disabled="updateBusy || update?.status !== 'verified'"
        @click="installUpdate"
      >
        {{ t("common.confirm") }}
      </button>
    </div></OrbitDialog
  >
  <p class="settings-footnote muted">
    Orivelle Orbit ·
    {{ formatBytes(store.state.settings.image_disk_limit_bytes) }}
  </p>
</template>
