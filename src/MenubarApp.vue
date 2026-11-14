<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import OrbitIcon from "./components/OrbitIcon.vue";
import { useMenubar } from "./composables/useMenubar";
import { nativeAvailable, subscribeMenubarLayout } from "./shared/bridge";
import {
  clientStatus,
  formatDecimal,
  formatMoney,
  formatUpdatedAt,
  nodeName,
} from "./shared/format";
import { locale, t, type MessageKey } from "./shared/i18n";
import type { Money } from "./shared/contracts";
import codexIcon from "./assets/codex-app.png";
import claudeIcon from "./assets/claude-desktop.png";
import "./menubar.css";

const panel = useMenubar();
const {
  state,
  account,
  connection,
  summary,
  activeTasks,
  attentionTask,
  usageFailed,
  notice,
  refreshing,
  now,
} = panel;
const surface = ref<HTMLElement>();
const arrow = ref(248);
const balance = computed(
  () =>
    account.value?.budget?.available ??
    (!account.value?.member ? account.value?.wallet : null),
);
const charges = computed(() => summary.value?.summary.charges ?? []);
const pending = computed(() =>
  charges.value.reduce((total, charge) => total + charge.pending_count, 0),
);
const initial = computed(
  () => account.value?.label.trim().slice(0, 1).toUpperCase() || "O",
);
const node = computed(() => {
  const index = state.value.nodes.findIndex(
    (item) => item.id === connection.value?.node_id,
  );
  return index < 0
    ? t("access.noNode")
    : nodeName(state.value.nodes[index]!, index);
});
const taskTitle = computed(() => {
  const keys: Record<string, MessageKey> = {
    queued: "menubar.queued",
    sending: "menubar.sending",
    waiting: "menubar.generating",
    saving: "images.saving",
    completed: "images.completed",
    save_failed: "images.saveFailed",
    unconfirmed: "images.unconfirmed",
  };
  return t(keys[attentionTask.value?.state ?? ""] ?? "menubar.idle");
});
const taskDetail = computed(() => {
  const task = attentionTask.value;
  if (!task) return t("menubar.idleHint");
  if (task.state === "save_failed") return t("menubar.saveHint");
  if (task.state === "unconfirmed") return t("menubar.unconfirmedHint");
  const label = task.requested_model_label || task.requested_model;
  const started = task.started_at ? Date.parse(task.started_at) : NaN;
  return activeTasks.value.length && Number.isFinite(started)
    ? `${label} · ${Math.max(0, Math.floor((now.value - started) / 1000))} ${t("menubar.seconds")}`
    : label;
});
function parts(value: Money | null | undefined, digits = 2) {
  const amount =
    value?.status === "known" && value.amount !== null
      ? formatDecimal(value.amount, digits)
      : null;
  return amount === null
    ? { currency: "", amount: formatMoney(value) }
    : { currency: value?.currency ?? t("common.unknown"), amount };
}
function chargeParts(charge: (typeof charges.value)[number]) {
  return parts(
    {
      amount: charge.known_amount ?? charge.amount,
      currency: charge.currency,
      status: "known",
    },
    4,
  );
}
const budget = computed(() => parts(balance.value));
function status(client: (typeof state.value.clients)[number]) {
  return clientStatus(client, connection.value, panel.initialized.value);
}
function keyboard(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.preventDefault();
    void panel.control({ action: "hide" });
  }
}
let observer: ResizeObserver | undefined;
let stopLayout: (() => void) | undefined;
let disposed = false;
let lastHeight = 0;
onMounted(async () => {
  window.addEventListener("keydown", keyboard);
  if (!nativeAvailable()) return;
  try {
    const stop = await subscribeMenubarLayout((offset) => {
      arrow.value = offset;
    });
    if (disposed) {
      stop();
      return;
    }
    stopLayout = stop;
    observer = new ResizeObserver(() => {
      const height = Math.ceil(
        surface.value?.getBoundingClientRect().height ?? 0,
      );
      if (height >= 100 && height !== lastHeight) {
        lastHeight = height;
        void panel.control({ action: "resize", height });
      }
    });
    if (surface.value) observer.observe(surface.value);
  } catch {
    notice.value = t("app.windowControlFailed");
  }
});
onUnmounted(() => {
  disposed = true;
  observer?.disconnect();
  stopLayout?.();
  window.removeEventListener("keydown", keyboard);
});
</script>

<template>
  <div
    ref="surface"
    class="menubar-surface"
    @pointerdown.self="panel.control({ action: 'hide' })"
  >
    <main
      class="menubar-panel"
      :style="{ '--menubar-arrow': `${arrow}px` }"
      :aria-label="t('menubar.label')"
    >
      <header class="mb-header">
        <div class="mb-brand">
          <span class="mb-brand-mark" aria-hidden="true"></span>
          <div>
            <strong>Orbit</strong>
            <p>
              <span class="mb-dot"></span
              >{{
                state.maintenance_active
                  ? t("common.updateMaintenance")
                  : t("menubar.running")
              }}
            </p>
          </div>
        </div>
        <button
          class="mb-icon-button"
          type="button"
          :aria-label="t('nav.settings')"
          @click="panel.open('settings')"
        >
          <OrbitIcon name="sliders" />
        </button>
      </header>
      <p v-if="notice" class="mb-notice" role="status">{{ notice }}</p>
      <section
        v-if="account"
        class="mb-account"
        :aria-label="t('home.account')"
      >
        <div class="mb-account-top">
          <div class="mb-person">
            <span class="mb-avatar">{{ initial }}</span
            ><span class="mb-person-name" :title="account.label">{{
              account.label
            }}</span>
          </div>
          <button
            class="mb-icon-button"
            type="button"
            :aria-label="t('action.refresh')"
            :disabled="refreshing || state.maintenance_active"
            :aria-busy="refreshing"
            @click="panel.refresh()"
          >
            <OrbitIcon name="refresh" />
          </button>
        </div>
        <div class="mb-amounts">
          <div>
            <p class="mb-amount-label">
              {{
                account.budget || account.member
                  ? t("home.budget")
                  : t("home.wallet")
              }}
            </p>
            <p class="mb-money">
              <span v-if="budget.currency" class="mb-currency">{{
                budget.currency
              }}</span
              ><strong>{{ budget.amount }}</strong>
            </p>
          </div>
          <div class="mb-spend">
            <p class="mb-amount-label">{{ t("menubar.todaySpend") }}</p>
            <template v-if="charges.length"
              ><p
                v-for="(charge, index) in charges"
                :key="index"
                class="mb-money mb-spend-money"
              >
                <span v-if="chargeParts(charge).currency" class="mb-currency">{{
                  chargeParts(charge).currency
                }}</span
                ><strong>{{ chargeParts(charge).amount }}</strong>
              </p></template
            >
            <p v-else class="mb-money mb-empty-money">
              {{
                summary?.summary.requests === 0
                  ? t("menubar.noSpend")
                  : t("common.unknown")
              }}
            </p>
          </div>
        </div>
        <div class="mb-account-bottom">
          <span :title="formatUpdatedAt(account.updated_at)"
            >{{ t("menubar.updated") }}
            {{
              account.updated_at &&
              Number.isFinite(Date.parse(account.updated_at))
                ? new Date(account.updated_at).toLocaleTimeString(locale, {
                    hour: "2-digit",
                    minute: "2-digit",
                  })
                : t("common.unknown")
            }}</span
          ><button class="mb-link" type="button" @click="panel.open('usage')">
            {{ t("menubar.platformUsage") }}<OrbitIcon name="chevron" />
          </button>
        </div>
        <p v-if="pending" class="mb-finance-hint">
          {{ pending }} {{ t("usage.pendingHint") }}
        </p>
        <p v-if="usageFailed" class="mb-finance-hint" role="status">
          {{ t("menubar.usageStale") }}
        </p>
      </section>
      <section v-else class="mb-account mb-login">
        <span class="mb-avatar">O</span>
        <div>
          <strong>{{ t("home.loggedOut") }}</strong>
          <p>{{ t("menubar.loginHint") }}</p>
        </div>
        <button class="mb-link" type="button" @click="panel.open('home')">
          {{ t("action.login") }}<OrbitIcon name="chevron" />
        </button>
      </section>
      <section class="mb-section">
        <div class="mb-section-title">
          <h2>{{ t("menubar.clients") }}</h2>
          <button class="mb-link" type="button" @click="panel.open('access')">
            {{ t("menubar.manage") }}<OrbitIcon name="external" />
          </button>
        </div>
        <button
          v-for="client in state.clients"
          :key="client.client"
          class="mb-client-row"
          type="button"
          @click="panel.open(client.client)"
        >
          <span class="mb-client-identity"
            ><img
              :src="client.client === 'codex' ? codexIcon : claudeIcon"
              alt=""
            /><span>{{
              client.client === "codex" ? "Codex" : "Claude Desktop"
            }}</span></span
          ><span
            class="mb-client-state"
            :class="`mb-tone-${status(client).tone}`"
            ><OrbitIcon
              v-if="status(client).tone === 'success'"
              name="check"
            />{{ status(client).text }}</span
          >
        </button>
        <button class="mb-node" type="button" @click="panel.open('access')">
          <span>{{ t("menubar.node") }}</span
          ><span class="mb-node-value"
            >{{ node }}<OrbitIcon name="chevron"
          /></span>
        </button>
      </section>
      <section class="mb-section mb-tasks">
        <div class="mb-section-title">
          <h2>{{ t("menubar.tasks") }}</h2>
          <span class="mb-task-count">{{
            activeTasks.length
              ? `${activeTasks.length} ${t("menubar.inProgress")}`
              : t("menubar.noTasks")
          }}</span>
        </div>
        <button class="mb-task-row" type="button" @click="panel.open('images')">
          <span class="mb-task-art"><OrbitIcon name="images" /></span
          ><span class="mb-task-info"
            ><strong>{{ taskTitle }}</strong
            ><span :title="taskDetail">{{ taskDetail }}</span></span
          ><OrbitIcon class="mb-task-chevron" name="chevron" />
        </button>
        <div
          v-if="activeTasks.length"
          class="mb-stages"
          :aria-label="taskTitle"
        >
          <span
            :class="{
              active:
                attentionTask?.state === 'queued' ||
                attentionTask?.state === 'sending',
            }"
            ><OrbitIcon
              :name="
                attentionTask?.state === 'queued' ||
                attentionTask?.state === 'sending'
                  ? 'loader'
                  : 'check'
              "
            />{{
              attentionTask?.state === "queued" ||
              attentionTask?.state === "sending"
                ? t("menubar.submittingStage")
                : t("menubar.submitted")
            }}</span
          ><i></i
          ><span :class="{ active: attentionTask?.state === 'waiting' }"
            ><span class="mb-dot"></span
            >{{ t("menubar.generatingStage") }}</span
          ><i></i
          ><span :class="{ active: attentionTask?.state === 'saving' }">{{
            t("menubar.saveStage")
          }}</span>
        </div>
      </section>
      <footer class="mb-footer">
        <button class="mb-open" type="button" @click="panel.open('home')">
          <OrbitIcon name="window" />{{ t("menubar.open")
          }}<OrbitIcon name="external" /></button
        ><button
          class="mb-icon-button"
          type="button"
          :aria-label="t('menubar.more')"
          @click="panel.control({ action: 'menu' })"
        >
          <OrbitIcon name="more" />
        </button>
      </footer>
    </main>
  </div>
</template>
