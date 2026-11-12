<script setup lang="ts">
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import { version } from "../../package.json";
import { useOrbitStore } from "../stores/orbit";
import type {
  DiagnosticsPreview,
  Locale,
  SettingsView,
  Theme,
} from "../shared/contracts";
import { redactServiceUrls } from "../shared/format";
import { t } from "../shared/i18n";
import OrbitDialog from "../components/OrbitDialog.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import StatusBadge from "../components/StatusBadge.vue";
const store = useOrbitStore();
const diagnostics = ref<DiagnosticsPreview | null>(null);
const logoutConnectionId = ref<string | null>(null);
const { update } = storeToRefs(store);
const installOpen = ref(false);
const updateBusy = computed(() => store.busy.has("update"));
const updateMessage = computed(() => {
  if (!store.initialized) return t("settings.notChecked");
  if (!store.inAppUpdates) return t("settings.manualUpdate");
  if (store.updateOperation === "check") return t("settings.checkingUpdate");
  if (store.updateOperation === "download")
    return t("settings.downloadingUpdate");
  if (store.updateOperation === "install")
    return t("settings.installingUpdate");
  if (store.updateCheckFailed) return t("settings.updateCheckFailed");
  if (update.value?.status === "up_to_date") return t("settings.upToDate");
  if (update.value?.status === "available")
    return `${t("settings.updateAvailable")} · ${update.value.version}`;
  if (update.value?.status === "verified")
    return `${t("settings.updateVerified")} · ${update.value.version}`;
  if (update.value?.status === "downloading")
    return t("settings.downloadingUpdate");
  if (update.value?.status === "blocked") return t("settings.updateBlocked");
  if (update.value?.status === "failed") return t("settings.updateFailed");
  return t("settings.notChecked");
});
const settingsDisabled = computed(
  () =>
    !store.native ||
    store.state.maintenance_active ||
    store.busy.has("settings"),
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
async function installUpdate() {
  const result = await store.installUpdate();
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
          {{ t("action.logout") }}
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
          :model-value="
            store.state.settings.close_behavior === 'quit'
              ? 'quit'
              : 'keep_running'
          "
          :disabled="settingsDisabled"
          :options="[
            { value: 'keep_running', label: t('settings.keepRunning') },
            { value: 'quit', label: t('settings.quit') },
          ]"
          @update:model-value="
            setting('close_behavior', $event as SettingsView['close_behavior'])
          "
        />
      </div>
      <p class="field-hint">{{ t("settings.backgroundHint") }}</p>
      <p class="field-hint">{{ t("settings.quitHint") }}</p>
    </section>
    <section class="settings-section">
      <div class="settings-section-title">
        <OrbitIcon name="download" />
        <h2>{{ t("settings.updates") }}</h2>
        <p class="settings-current-version">
          <span>{{ t("settings.currentVersion") }}</span>
          <strong>{{ version }}</strong>
        </p>
      </div>
      <label v-if="store.inAppUpdates" class="checkbox-row"
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
      <p class="muted update-status" role="status" aria-live="polite">
        {{ updateMessage }}
      </p>
      <div v-if="store.inAppUpdates" class="settings-actions">
        <button
          class="button secondary"
          :disabled="updateBusy || store.state.maintenance_active"
          @click="store.checkUpdates()"
        >
          {{ t("settings.check") }}</button
        ><button
          v-if="update?.status === 'available'"
          class="button primary"
          :disabled="updateBusy"
          @click="store.downloadUpdate()"
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
</template>
