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
  <p v-if="store.account" class="field-hint account-updated">
    {{ t("home.updatedAt") }} · {{ formatUpdatedAt(store.account.updated_at) }}
    <span v-if="store.state.session.status === 'refreshing'">
      · {{ t("login.refreshing") }}</span
    >
  </p>
  <section class="account-strip" :aria-label="t('home.account')">
    <div class="home-account-identity">
      <span class="account-avatar" aria-hidden="true">
        <OrbitIcon name="account" />
      </span>
      <div class="account-summary">
        <h2>{{ store.account?.label ?? t("home.loggedOut") }}</h2>
        <p class="muted">
          {{ store.account?.email ?? t("home.loggedOutHint") }}
        </p>
      </div>
    </div>
    <dl
      v-if="store.account && (!store.account.member || store.account.budget)"
      class="account-balances"
    >
      <div v-if="!store.account.member" class="account-balance">
        <dt>{{ t("home.wallet") }}</dt>
        <dd>{{ formatMoney(store.account.wallet, 4) }}</dd>
      </div>
      <div v-if="store.account.budget" class="account-balance">
        <dt>{{ t("home.budget") }}</dt>
        <dd>{{ formatMoney(store.account.budget.available) }}</dd>
      </div>
    </dl>
    <button
      v-else-if="!store.account && store.canLogin"
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
