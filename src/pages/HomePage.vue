<script setup lang="ts">
import { useOrbitStore } from "../stores/orbit";
import { t } from "../shared/i18n";
import { formatMoney, formatUpdatedAt } from "../shared/format";
import ClientRow from "../components/ClientRow.vue";
import OrbitIcon from "../components/OrbitIcon.vue";
const store = useOrbitStore();
</script>
<template>
  <section class="page-heading">
    <h1>{{ t("home.title") }}</h1>
    <p class="page-description">{{ t("home.description") }}</p>
  </section>
  <p v-if="store.account" class="field-hint">
    {{ t("home.updatedAt") }} · {{ formatUpdatedAt(store.account.updated_at) }}
    <span v-if="store.state.session.status === 'refreshing'">
      · {{ t("login.refreshing") }}</span
    >
  </p>
  <section class="account-strip" :aria-label="t('home.account')">
    <span class="account-avatar" aria-hidden="true"
      ><OrbitIcon name="account"
    /></span>
    <div class="account-summary">
      <h2>{{ store.account?.label ?? t("home.loggedOut") }}</h2>
      <p class="muted">{{ store.account?.email ?? t("home.loggedOutHint") }}</p>
    </div>
    <template v-if="store.account"
      ><div v-if="!store.account.member" class="balance-inline">
        <span>{{ t("home.wallet") }}</span
        ><strong>{{ formatMoney(store.account.wallet, 4) }}</strong>
      </div>
      <div v-if="store.account.budget" class="balance-inline">
        <span>{{ t("home.budget") }}</span
        ><strong>{{ formatMoney(store.account.budget.available) }}</strong>
      </div></template
    ><button
      v-else-if="store.canLogin"
      class="button primary"
      @click="store.loginOpen = true"
    >
      <OrbitIcon name="plus" />{{ t("action.login") }}
    </button>
  </section>
  <p
    v-if="
      store.state.session.inference_credential_status === 'permission_denied'
    "
    class="info-box"
  >
    <OrbitIcon name="shield" />{{ t("home.permissionDenied") }}
  </p>
  <section class="client-list">
    <ClientRow
      v-for="client in store.state.clients"
      :key="client.client"
      :client="client"
      :connection="store.activeConnection"
      :detected="store.initialized"
    />
  </section>
  <div class="home-footnote">
    <OrbitIcon name="shield" />
    <p>{{ t("access.previewHint") }}</p>
  </div>
  <RouterLink class="image-entry" to="/images"
    ><span class="image-entry-art" aria-hidden="true"
      ><OrbitIcon name="images" /></span
    ><span
      ><strong>{{ t("home.images") }}</strong
      ><span class="muted">{{ t("home.imagesHint") }}</span></span
    ><OrbitIcon name="arrow"
  /></RouterLink>
</template>
