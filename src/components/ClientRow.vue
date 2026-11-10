<script setup lang="ts">
import { computed } from "vue";
import type { ClientView, ConnectionView } from "../shared/contracts";
import { clientStatus, evidenceMatches } from "../shared/format";
import { t } from "../shared/i18n";
import codexIcon from "../assets/codex-app.png";
import claudeIcon from "../assets/claude-desktop.png";
import StatusBadge from "./StatusBadge.vue";
import OrbitIcon from "./OrbitIcon.vue";
const props = defineProps<{
  client: ClientView;
  connection: ConnectionView | null;
  detected: boolean;
  compact?: boolean;
}>();
const status = computed(() =>
  clientStatus(props.client, props.connection, props.detected),
);
const configurationAccess = computed(() => {
  if (
    props.client.managed === "yes" ||
    props.client.configuration_access === "blocked"
  )
    return "client.configurationBlocked";
  return props.client.configuration_access === "allowed" &&
    props.client.configuration_access_revision
    ? "client.configurationAllowed"
    : "client.configurationUnverified";
});
const loaded = computed(
  () =>
    props.client.loaded_state === "confirmed" &&
    evidenceMatches(
      props.client.loaded_evidence,
      props.client,
      props.connection,
    ),
);
const verified = computed(
  () =>
    props.client.connection_state === "passed" &&
    evidenceMatches(
      props.client.connection_evidence,
      props.client,
      props.connection,
    ),
);
</script>
<template>
  <div class="client-row" :class="{ 'client-row-compact': compact }">
    <img
      class="client-symbol"
      :src="client.client === 'codex' ? codexIcon : claudeIcon"
      alt=""
    />
    <div class="client-info">
      <div class="client-title-line">
        <h2>
          {{ t(client.client === "codex" ? "home.codex" : "home.claude") }}
        </h2>
        <span v-if="client.version" class="version-label">{{
          client.version
        }}</span>
      </div>
      <p class="muted">
        {{
          t(client.client === "codex" ? "home.codexHint" : "home.claudeHint")
        }}
      </p>
      <p class="muted">
        {{ t(configurationAccess) }}
      </p>
      <p v-if="client.managed === 'unknown'" class="muted">
        {{ t("client.remotePolicyUnknown") }}
      </p>
      <div
        v-if="!compact && client.configuration_state === 'saved'"
        class="client-stages"
      >
        <StatusBadge :text="t('client.saved')" /><StatusBadge
          :text="t(loaded ? 'client.loaded' : 'client.restart')"
          :tone="loaded ? 'success' : 'warning'"
        /><StatusBadge
          :text="t(verified ? 'client.verified' : 'client.notTested')"
          :tone="verified ? 'success' : 'neutral'"
        />
      </div>
    </div>
    <StatusBadge :text="status.text" :tone="status.tone" /><slot
      ><RouterLink
        v-if="!compact"
        class="button secondary"
        :to="{ path: '/access', hash: `#client-${client.client}` }"
        >{{ t("action.manage") }}<OrbitIcon name="arrow" /></RouterLink
    ></slot>
  </div>
</template>
