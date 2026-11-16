<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "reka-ui";
import OrbitIcon from "../../src/components/OrbitIcon.vue";
import OrbitDialog from "../../src/components/OrbitDialog.vue";
import OrbitSelect from "../../src/components/OrbitSelect.vue";
import StatusBadge from "../../src/components/StatusBadge.vue";

type Status = "active" | "exhausted" | "expired" | "revoked";
type Tier = "follow_client" | "off" | "priority" | "ultrafast";
type LimitMode = "unlimited" | "cumulative" | "rolling";
type ApiKey = {
  id: string;
  name: string;
  prefix: string;
  status: Status;
  used: number;
  limit: number | null;
  mode: LimitMode;
  window: string;
  tier: Tier;
  expires: string;
  lastUsed: string;
  created: string;
};
type Action = "rotate" | "reset" | "revoke" | "delete";

const initialKeys = (): ApiKey[] => [
  {
    id: "daily",
    name: "日常开发",
    prefix: "orv_sk_7a3f…k9m2",
    status: "active",
    used: 18.42,
    limit: 50,
    mode: "cumulative",
    window: "300",
    tier: "follow_client",
    expires: "2026-12-31",
    lastUsed: "刚刚",
    created: "2026-11-02 09:30",
  },
  {
    id: "orbit",
    name: "Orbit · MacBook Pro",
    prefix: "orv_sk_2c8e…p4x7",
    status: "active",
    used: 6.8,
    limit: null,
    mode: "unlimited",
    window: "300",
    tier: "off",
    expires: "",
    lastUsed: "12 分钟前",
    created: "2026-11-10 14:20",
  },
  {
    id: "images",
    name: "图片创作",
    prefix: "orv_sk_9f6b…t2r5",
    status: "active",
    used: 8.16,
    limit: 20,
    mode: "rolling",
    window: "1440",
    tier: "follow_client",
    expires: "",
    lastUsed: "今天 10:24",
    created: "2026-11-05 16:45",
  },
  {
    id: "test",
    name: "临时测试",
    prefix: "orv_sk_4d1a…w6n3",
    status: "exhausted",
    used: 5,
    limit: 5,
    mode: "cumulative",
    window: "300",
    tier: "priority",
    expires: "2026-11-30",
    lastUsed: "昨天 18:42",
    created: "2026-11-12 11:10",
  },
  {
    id: "old",
    name: "旧设备",
    prefix: "orv_sk_8e5c…j3h6",
    status: "revoked",
    used: 12.34,
    limit: 30,
    mode: "cumulative",
    window: "300",
    tier: "off",
    expires: "",
    lastUsed: "11 月 08 日",
    created: "2026-10-18 08:50",
  },
];
const keys = ref(initialKeys());
const defaultId = ref("daily");
const bound = ref({ codex: "daily", claude: "orbit" });
const search = ref("");
const filter = ref("all");
const dark = ref(false);
const readOnly = ref(false);
const empty = ref(false);
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let revealTimer: ReturnType<typeof setTimeout> | undefined;
let serial = 0;

const statusLabels: Record<Status, string> = {
  active: "有效",
  exhausted: "额度用尽",
  expired: "已过期",
  revoked: "已撤销",
};
const tierOptions: { value: Tier; label: string }[] = [
  { value: "follow_client", label: "跟随客户端" },
  { value: "off", label: "普通速度" },
  { value: "priority", label: "Fast" },
  { value: "ultrafast", label: "Ultrafast" },
];
const windowOptions = [
  { value: "300", label: "5 小时" },
  { value: "1440", label: "1 天" },
  { value: "10080", label: "7 天" },
];
const visibleKeys = computed(() => (empty.value ? [] : keys.value));
const filtered = computed(() =>
  visibleKeys.value.filter(
    (key) =>
      (filter.value === "all" || key.status === filter.value) &&
      `${key.name} ${key.prefix}`
        .toLowerCase()
        .includes(search.value.trim().toLowerCase()),
  ),
);
const current = computed(() =>
  visibleKeys.value.find((key) => key.id === defaultId.value),
);
const usable = computed(() =>
  visibleKeys.value.filter((key) => key.status === "active"),
);
const switchOpen = ref(false);
const selectedId = ref(defaultId.value);
const detailsOpen = ref(false);
const detailsId = ref("daily");
const detail = computed(() =>
  keys.value.find((key) => key.id === detailsId.value),
);
const editorOpen = ref(false);
const editingId = ref<string | null>(null);
const form = ref({
  name: "",
  expires: "",
  tier: "follow_client" as Tier,
  mode: "unlimited" as LimitMode,
  limit: "",
  window: "300",
  use: true,
});
const confirmOpen = ref(false);
const pending = ref<{ action: Action; id: string } | null>(null);
const pendingKey = computed(() =>
  keys.value.find((key) => key.id === pending.value?.id),
);
const revealOpen = ref(false);
const revealKey = ref<ApiKey | null>(null);
const revealed = ref(false);
const updateClient = ref<"codex" | "claude" | null>(null);
const updateOpen = ref(false);
const clientLabels = { codex: "Codex", claude: "Claude Desktop" };
const actions: Record<
  Action,
  { title: string; verb: string; description: string }
> = {
  rotate: {
    title: "轮换 API Key",
    verb: "确认轮换",
    description:
      "将生成新的密钥，旧密钥会立即失效。所有使用这把密钥的工具都需要更新。",
  },
  reset: {
    title: "重置消费计数",
    verb: "确认重置",
    description:
      "重置这把密钥的消费计数；余额、历史调用和实际账单保持原有记录。",
  },
  revoke: {
    title: "撤销 API Key",
    verb: "确认撤销",
    description:
      "这把密钥将无法再发起新调用。所有使用它的工具和设备都会受到影响。",
  },
  delete: {
    title: "删除 API Key",
    verb: "确认删除",
    description: "删除后将无法恢复这把密钥。历史调用和消费记录继续保留。",
  },
};
const pendingAction = computed(
  () => actions[pending.value?.action ?? "revoke"],
);
const money = (value: number) => `$${value.toFixed(2)}`;
const tierLabel = (tier: Tier) =>
  tierOptions.find((option) => option.value === tier)?.label;
const keyName = (id: string) =>
  keys.value.find((key) => key.id === id)?.name ?? "未绑定";
const tone = (status: Status) =>
  status === "active"
    ? "success"
    : status === "revoked"
      ? "neutral"
      : "warning";
const percent = (key: ApiKey) =>
  key.limit ? Math.min(100, (key.used / key.limit) * 100) : 0;
function notify(message: string) {
  clearTimeout(toastTimer);
  toast.value = message;
  toastTimer = setTimeout(() => {
    toast.value = "";
  }, 3800);
}
function inspect(key: ApiKey) {
  detailsId.value = key.id;
  detailsOpen.value = true;
}
function choose(key: ApiKey) {
  if (readOnly.value || key.status !== "active") return;
  defaultId.value = key.id;
  switchOpen.value = false;
  notify(`已将“${key.name}”设为默认密钥`);
}
function openSwitch() {
  selectedId.value = current.value?.id ?? usable.value[0]?.id ?? "";
  switchOpen.value = true;
}
function saveSelection() {
  const key = keys.value.find((key) => key.id === selectedId.value);
  if (key) choose(key);
}
function edit(key?: ApiKey) {
  detailsOpen.value = false;
  editingId.value = key?.id ?? null;
  form.value = {
    name: key?.name ?? "",
    expires: key?.expires ?? "",
    tier: key?.tier ?? "follow_client",
    mode: key?.mode ?? "unlimited",
    limit: key?.limit?.toString() ?? "",
    window: key?.window ?? "300",
    use: true,
  };
  editorOpen.value = true;
}
function save() {
  if (readOnly.value || !form.value.name.trim()) return;
  const data = {
    name: form.value.name.trim(),
    expires: form.value.expires,
    tier: form.value.tier,
    mode: form.value.mode,
    limit: form.value.mode === "unlimited" ? null : Number(form.value.limit),
    window: form.value.window,
  };
  if (editingId.value) {
    const key = keys.value.find((key) => key.id === editingId.value);
    if (key) {
      Object.assign(key, data);
      if (
        key.status === "exhausted" &&
        (key.limit === null || key.used < key.limit)
      )
        key.status = "active";
    }
    notify("密钥设置已保存");
  } else {
    const id = `new-${++serial}`;
    keys.value.unshift({
      id,
      ...data,
      prefix: `orv_sk_demo…${String(serial).padStart(4, "0")}`,
      status: "active",
      used: 0,
      lastUsed: "尚未使用",
      created: "2026-11-16 14:30",
    });
    empty.value = false;
    filter.value = "all";
    search.value = "";
    if (form.value.use) defaultId.value = id;
    notify(form.value.use ? "已创建密钥并设为 Orbit 默认" : "已创建密钥");
  }
  editorOpen.value = false;
}
function requestAction(action: Action, key: ApiKey) {
  detailsOpen.value = false;
  pending.value = { action, id: key.id };
  confirmOpen.value = true;
}
function confirm() {
  if (!pending.value || !pendingKey.value || readOnly.value) return;
  const key = pendingKey.value;
  const action = pending.value.action;
  if (action === "reset") {
    key.used = 0;
    if (key.status === "exhausted") key.status = "active";
  }
  if (action === "revoke") key.status = "revoked";
  if (action === "delete")
    keys.value = keys.value.filter((item) => item.id !== key.id);
  if (action === "rotate") {
    key.prefix = `orv_sk_demo…r${++serial}26`;
    if (bound.value.codex === key.id) bound.value.codex = "rotated";
    if (bound.value.claude === key.id) bound.value.claude = "rotated";
  }
  confirmOpen.value = false;
  notify(
    {
      reset: "已重置消费计数",
      revoke: "已撤销密钥",
      delete: "已删除密钥",
      rotate: "已轮换密钥，请更新使用它的客户端",
    }[action],
  );
}
function openReveal(key: ApiKey) {
  detailsOpen.value = false;
  revealKey.value = key;
  revealed.value = false;
  revealOpen.value = true;
}
function reveal() {
  revealed.value = true;
  clearTimeout(revealTimer);
  revealTimer = setTimeout(() => {
    revealed.value = false;
  }, 30000);
}
function copy() {
  notify("已模拟复制演示密钥");
}
function requestUpdate(client: "codex" | "claude") {
  updateClient.value = client;
  updateOpen.value = true;
}
function applyUpdate() {
  if (updateClient.value && current.value?.status === "active") {
    bound.value[updateClient.value] = current.value.id;
    notify(`${clientLabels[updateClient.value]} 接入已更新，重启客户端后生效`);
  }
  updateOpen.value = false;
}
function reset() {
  keys.value = initialKeys();
  defaultId.value = "daily";
  bound.value = { codex: "daily", claude: "orbit" };
  search.value = "";
  filter.value = "all";
  empty.value = false;
  readOnly.value = false;
  notify("预览已恢复初始状态");
}
watch(dark, (value) =>
  document.documentElement.classList.toggle("dark", value),
);
watch(revealOpen, (open) => {
  if (!open) {
    revealed.value = false;
    revealKey.value = null;
    clearTimeout(revealTimer);
  }
});
function hideSecret() {
  revealed.value = false;
}
window.addEventListener("blur", hideSecret);
onUnmounted(() => {
  clearTimeout(toastTimer);
  clearTimeout(revealTimer);
  window.removeEventListener("blur", hideSecret);
});
</script>

<template>
  <header class="topbar preview-topbar">
    <span class="topbar-subtitle">Orivelle Orbit</span>
    <div class="topbar-drag" />
    <span class="preview-label">界面预览</span>
    <button
      class="icon-button"
      :aria-label="dark ? '切换浅色' : '切换深色'"
      :title="dark ? '切换浅色' : '切换深色'"
      @click="dark = !dark"
    >
      <OrbitIcon name="sun" />
    </button>
    <button
      class="icon-button"
      aria-label="重置预览"
      title="重置预览"
      @click="reset"
    >
      <OrbitIcon name="refresh" />
    </button>
  </header>
  <div class="app-shell">
    <aside class="sidebar">
      <a class="brand" href="#api-keys"
        ><span class="brand-mark" aria-hidden="true" /><span
          >Orivelle <strong>Orbit</strong></span
        ></a
      >
      <nav aria-label="主要导航">
        <button
          v-for="item in [
            { icon: 'home', label: '首页' },
            { icon: 'images', label: '图片工作台' },
            { icon: 'access', label: '接入' },
            { icon: 'key', label: 'API Key' },
            { icon: 'chart', label: '用量' },
            { icon: 'settings', label: '设置' },
          ]"
          :key="item.label"
          class="nav-link preview-nav"
          :class="{ 'nav-active': item.icon === 'key' }"
          :aria-current="item.icon === 'key' ? 'page' : undefined"
          @click="item.icon !== 'key' && notify('当前预览展示 API Key 页面')"
        >
          <OrbitIcon :name="item.icon" /><span>{{ item.label }}</span>
        </button>
      </nav>
      <div class="sidebar-bottom">
        <span class="connection-dot connected" /><span class="truncate"
          >Orbit 演示账户</span
        ><OrbitIcon name="account" :size="16" />
      </div>
    </aside>
    <div class="main-shell">
      <main id="main-content" class="page-container key-page" tabindex="-1">
        <section class="page-heading key-heading">
          <div>
            <h1>API Key</h1>
            <p class="page-description">
              管理访问凭据，选择 Orbit 使用的密钥。
            </p>
          </div>
          <button class="button primary" :disabled="readOnly" @click="edit()">
            <OrbitIcon name="plus" :size="16" />创建密钥
          </button>
        </section>

        <p v-if="readOnly" class="key-readonly">
          <OrbitIcon
            name="shield"
            :size="16"
          />当前账户为只读，可查看密钥信息，暂不能创建、修改或切换密钥。
        </p>

        <section
          v-if="visibleKeys.length"
          class="key-current"
          aria-label="Orbit 当前默认密钥"
        >
          <div class="key-current-main">
            <div class="key-symbol"><OrbitIcon name="key" :size="23" /></div>
            <div class="key-current-info">
              <p class="key-caption">Orbit 当前默认</p>
              <div class="key-current-title">
                <h2>{{ current?.name ?? "请选择密钥" }}</h2>
                <StatusBadge
                  v-if="current"
                  :tone="tone(current.status)"
                  :text="statusLabels[current.status]"
                />
              </div>
              <p class="key-prefix mono">
                {{ current?.prefix ?? "选择一把有效密钥后即可开始使用" }}
              </p>
            </div>
            <button
              class="button secondary key-switch"
              :disabled="readOnly || !usable.length"
              @click="openSwitch"
            >
              切换密钥<OrbitIcon name="chevron-down" :size="14" />
            </button>
          </div>
          <div class="key-bindings">
            <div
              v-for="client in ['codex', 'claude'] as const"
              :key="client"
              class="key-binding"
            >
              <span class="key-client-name">{{ clientLabels[client] }}</span
              ><span class="key-bound-name">{{
                bound[client] === "rotated"
                  ? "密钥已轮换"
                  : keyName(bound[client])
              }}</span
              ><span v-if="bound[client] === defaultId" class="key-binding-ok"
                ><OrbitIcon name="check" :size="13" />已绑定</span
              ><button
                v-else
                class="text-button key-update"
                :disabled="readOnly || current?.status !== 'active'"
                @click="requestUpdate(client)"
              >
                更新接入<OrbitIcon name="arrow" :size="13" />
              </button>
            </div>
          </div>
          <p class="key-current-note">
            默认密钥用于新建图片任务与后续接入；已接入的工具分别保留当前绑定。
          </p>
        </section>

        <section class="key-list" aria-label="我的密钥">
          <div class="key-list-heading">
            <h2>
              我的密钥<span class="key-count">{{ visibleKeys.length }}</span>
            </h2>
            <button
              class="text-button key-refresh"
              @click="notify('密钥信息已刷新')"
            >
              <OrbitIcon name="refresh" :size="14" />刷新
            </button>
          </div>
          <div v-if="visibleKeys.length" class="key-toolbar">
            <div class="key-search">
              <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
                <circle cx="10.5" cy="10.5" r="6.5" />
                <path d="m16 16 4.5 4.5" /></svg
              ><input
                v-model="search"
                aria-label="搜索密钥"
                placeholder="搜索名称或密钥前缀"
                type="search"
              />
            </div>
            <OrbitSelect
              id="key-status"
              v-model="filter"
              :options="[
                { value: 'all', label: '全部状态' },
                { value: 'active', label: '有效' },
                { value: 'exhausted', label: '额度用尽' },
                { value: 'expired', label: '已过期' },
                { value: 'revoked', label: '已撤销' },
              ]"
            />
          </div>
          <div v-if="filtered.length" class="key-table-wrap">
            <table class="key-table">
              <thead>
                <tr>
                  <th>密钥名称</th>
                  <th>状态</th>
                  <th>消费 / 限额</th>
                  <th class="key-tier-column">速度策略</th>
                  <th>有效期</th>
                  <th><span class="sr-only">操作</span></th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="key in filtered"
                  :key="key.id"
                  :class="{ 'key-row-muted': key.status === 'revoked' }"
                >
                  <td class="key-name-cell">
                    <button class="key-name-button" @click="inspect(key)">
                      {{ key.name
                      }}<span
                        v-if="key.id === defaultId"
                        class="key-default-label"
                        >默认</span
                      ></button
                    ><span class="key-prefix mono">{{ key.prefix }}</span>
                  </td>
                  <td class="key-status-cell">
                    <StatusBadge
                      :tone="tone(key.status)"
                      :text="statusLabels[key.status]"
                    />
                  </td>
                  <td class="key-spending-cell">
                    <div class="key-spending">
                      <span>{{ money(key.used) }}</span
                      ><span class="key-spending-limit"
                        >/
                        {{
                          key.limit === null ? "不限额" : money(key.limit)
                        }}</span
                      >
                    </div>
                    <div
                      v-if="key.limit"
                      class="key-progress"
                      :class="{ exhausted: key.status === 'exhausted' }"
                    >
                      <span :style="{ width: `${percent(key)}%` }" />
                    </div>
                    <span v-else class="key-table-subtle">按账户可用余额</span
                    ><span
                      v-if="key.mode === 'rolling'"
                      class="key-table-subtle"
                      >{{
                        windowOptions.find(
                          (option) => option.value === key.window,
                        )?.label
                      }}滚动窗口</span
                    >
                  </td>
                  <td class="key-tier-column">{{ tierLabel(key.tier) }}</td>
                  <td class="key-expiry-cell">
                    <span>{{
                      key.expires
                        ? key.expires.replaceAll("-", "/")
                        : "长期有效"
                    }}</span>
                  </td>
                  <td class="key-actions">
                    <button class="text-button" @click="inspect(key)">
                      详情</button
                    ><DropdownMenuRoot
                      ><DropdownMenuTrigger
                        class="icon-button"
                        :aria-label="`${key.name}的更多操作`"
                        ><OrbitIcon
                          name="more"
                          :size="17" /></DropdownMenuTrigger
                      ><DropdownMenuPortal
                        ><DropdownMenuContent
                          class="key-menu"
                          align="end"
                          :side-offset="5"
                          :collision-padding="12"
                          ><DropdownMenuItem
                            class="key-menu-item"
                            :disabled="
                              readOnly ||
                              key.status !== 'active' ||
                              key.id === defaultId
                            "
                            @select="choose(key)"
                            ><OrbitIcon name="check" :size="15" />设为 Orbit
                            默认</DropdownMenuItem
                          ><DropdownMenuItem
                            class="key-menu-item"
                            :disabled="readOnly || key.status === 'revoked'"
                            @select="edit(key)"
                            ><OrbitIcon
                              name="edit"
                              :size="15"
                            />编辑密钥</DropdownMenuItem
                          ><DropdownMenuItem
                            class="key-menu-item"
                            :disabled="key.status === 'revoked'"
                            @select="openReveal(key)"
                            ><OrbitIcon
                              name="key"
                              :size="15"
                            />查看与复制</DropdownMenuItem
                          ><DropdownMenuItem
                            class="key-menu-item"
                            :disabled="readOnly || key.status === 'revoked'"
                            @select="requestAction('rotate', key)"
                            ><OrbitIcon
                              name="refresh"
                              :size="15"
                            />轮换密钥</DropdownMenuItem
                          ><DropdownMenuItem
                            class="key-menu-item"
                            :disabled="readOnly || key.status === 'revoked'"
                            @select="requestAction('reset', key)"
                            ><OrbitIcon
                              name="history"
                              :size="15"
                            />重置消费计数</DropdownMenuItem
                          ><DropdownMenuSeparator
                            class="key-menu-separator"
                          /><DropdownMenuItem
                            class="key-menu-item danger"
                            :disabled="readOnly || key.status === 'revoked'"
                            @select="requestAction('revoke', key)"
                            ><OrbitIcon
                              name="close"
                              :size="15"
                            />撤销密钥</DropdownMenuItem
                          ><DropdownMenuItem
                            class="key-menu-item danger"
                            :disabled="readOnly"
                            @select="requestAction('delete', key)"
                            ><OrbitIcon
                              name="trash"
                              :size="15"
                            />删除密钥</DropdownMenuItem
                          ></DropdownMenuContent
                        ></DropdownMenuPortal
                      ></DropdownMenuRoot
                    >
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <div v-else class="key-empty">
            <div class="key-empty-symbol">
              <OrbitIcon name="key" :size="27" />
            </div>
            <h3>
              {{
                visibleKeys.length ? "没有找到匹配的密钥" : "创建你的第一把密钥"
              }}
            </h3>
            <p>
              {{
                visibleKeys.length
                  ? "试试其他名称，或调整状态筛选。"
                  : "一把密钥即可访问账户获准的模型，也可为不同用途分别设置限额。"
              }}
            </p>
            <button
              v-if="visibleKeys.length"
              class="button secondary"
              @click="
                search = '';
                filter = 'all';
              "
            >
              清除筛选</button
            ><button
              v-else
              class="button primary"
              :disabled="readOnly"
              @click="edit()"
            >
              <OrbitIcon name="plus" :size="15" />创建密钥
            </button>
          </div>
          <div v-if="filtered.length" class="key-table-footer">
            <span>显示 {{ filtered.length }} 把密钥</span
            ><span>金额单位 USD</span>
          </div>
        </section>
        <footer class="key-preview-footer">
          <span>示例数据 · 修改仅在当前预览中生效</span>
          <div>
            <label><input v-model="readOnly" type="checkbox" />只读账户</label
            ><label><input v-model="empty" type="checkbox" />空状态</label>
          </div>
        </footer>
      </main>
    </div>
  </div>

  <OrbitDialog
    v-model:open="switchOpen"
    title="选择默认密钥"
    description="新建图片任务和后续接入将使用此密钥。已有工具需要分别更新接入。"
    ><div class="key-choice-list">
      <label
        v-for="key in usable"
        :key="key.id"
        class="key-choice"
        :class="{ selected: key.id === selectedId }"
        ><input
          v-model="selectedId"
          type="radio"
          name="default-key"
          :value="key.id"
        />
        <div>
          <strong>{{ key.name }}</strong
          ><span class="key-prefix mono">{{ key.prefix }}</span>
        </div>
        <span v-if="key.id === defaultId" class="key-choice-current"
          >当前默认</span
        ></label
      >
    </div>
    <div class="dialog-actions">
      <button class="button secondary" @click="switchOpen = false">取消</button
      ><button
        class="button primary"
        :disabled="!selectedId || readOnly"
        @click="saveSelection"
      >
        使用此密钥
      </button>
    </div></OrbitDialog
  >

  <OrbitDialog
    v-model:open="detailsOpen"
    :title="detail?.name ?? '密钥详情'"
    description="查看密钥状态、消费与接入信息。"
    ><template v-if="detail"
      ><div class="key-detail-secret">
        <code>{{ detail.prefix }}</code
        ><button
          class="text-button"
          :disabled="detail.status === 'revoked'"
          @click="openReveal(detail)"
        >
          查看与复制<OrbitIcon name="external" :size="13" />
        </button>
      </div>
      <dl class="key-detail-grid">
        <div>
          <dt>状态</dt>
          <dd>
            <StatusBadge
              :tone="tone(detail.status)"
              :text="statusLabels[detail.status]"
            />
          </dd>
        </div>
        <div>
          <dt>速度策略</dt>
          <dd>{{ tierLabel(detail.tier) }}</dd>
        </div>
        <div>
          <dt>有效期</dt>
          <dd>{{ detail.expires || "长期有效" }}</dd>
        </div>
        <div>
          <dt>最近使用</dt>
          <dd>{{ detail.lastUsed }}</dd>
        </div>
        <div>
          <dt>创建时间</dt>
          <dd>{{ detail.created }}</dd>
        </div>
        <div>
          <dt>额度模式</dt>
          <dd>
            {{
              detail.mode === "rolling"
                ? `${windowOptions.find((option) => option.value === detail?.window)?.label}滚动窗口`
                : detail.mode === "unlimited"
                  ? "不限额"
                  : "累计额度"
            }}
          </dd>
        </div>
      </dl>
      <div class="key-detail-spending">
        <span>已用金额</span
        ><strong
          >{{ money(detail.used)
          }}<small
            >/
            {{ detail.limit === null ? "不限额" : money(detail.limit) }}</small
          ></strong
        >
        <div
          v-if="detail.limit"
          class="key-progress"
          :class="{ exhausted: detail.status === 'exhausted' }"
        >
          <span :style="{ width: `${percent(detail)}%` }" />
        </div>
        <button
          class="text-button"
          @click="notify('正式接入后，将打开此密钥的用量记录')"
        >
          查看用量记录<OrbitIcon name="arrow" :size="13" />
        </button>
      </div>
      <div class="key-detail-bindings">
        <p class="key-caption">本机使用情况</p>
        <p v-if="detail.id === defaultId">
          <OrbitIcon name="check" :size="14" />Orbit 默认密钥
        </p>
        <p v-if="detail.id === bound.codex">
          <OrbitIcon name="window" :size="14" />Codex
        </p>
        <p v-if="detail.id === bound.claude">
          <OrbitIcon name="window" :size="14" />Claude Desktop
        </p>
        <p
          v-if="
            detail.id !== defaultId &&
            detail.id !== bound.codex &&
            detail.id !== bound.claude
          "
          class="muted"
        >
          暂无本机接入绑定
        </p>
      </div>
      <div class="dialog-actions">
        <button
          class="button secondary"
          :disabled="readOnly || detail.status === 'revoked'"
          @click="edit(detail)"
        >
          <OrbitIcon name="edit" :size="14" />编辑</button
        ><button
          class="button primary"
          :disabled="
            readOnly || detail.status !== 'active' || detail.id === defaultId
          "
          @click="
            choose(detail);
            detailsOpen = false;
          "
        >
          {{ detail.id === defaultId ? "当前默认密钥" : "设为 Orbit 默认" }}
        </button>
      </div></template
    ></OrbitDialog
  >

  <OrbitDialog
    v-model:open="editorOpen"
    :title="editingId ? '编辑密钥' : '创建 API Key'"
    description="可用于账户获准的模型，消费遵循账户余额、预算与密钥限额。"
    ><form class="key-form" @submit.prevent="save">
      <div class="field">
        <label class="field-label" for="key-name">密钥名称</label
        ><input
          id="key-name"
          v-model="form.name"
          placeholder="例如：日常开发、图片创作"
          maxlength="100"
          required
        />
      </div>
      <div class="key-form-columns">
        <div class="field">
          <label class="field-label" for="key-expiry">有效期</label
          ><input
            id="key-expiry"
            v-model="form.expires"
            type="date"
            min="2026-11-17"
          />
          <p class="field-hint">留空则长期有效</p>
        </div>
        <div class="field">
          <label class="field-label" for="key-tier">速度策略</label
          ><OrbitSelect
            id="key-tier"
            v-model="form.tier"
            :options="tierOptions"
          />
          <p class="field-hint">按模型支持情况执行</p>
        </div>
      </div>
      <div class="key-form-limit">
        <div class="field">
          <label class="field-label" for="key-mode">金额限额</label
          ><OrbitSelect
            id="key-mode"
            v-model="form.mode"
            :options="[
              { value: 'unlimited', label: '不限额' },
              { value: 'cumulative', label: '累计额度' },
              { value: 'rolling', label: '滚动窗口' },
            ]"
          />
        </div>
        <div v-if="form.mode !== 'unlimited'" class="key-form-columns">
          <div class="field">
            <label class="field-label" for="key-limit"
              >限额金额 <span class="key-unit">USD</span></label
            ><input
              id="key-limit"
              v-model="form.limit"
              type="number"
              min="0.01"
              step="0.01"
              placeholder="50.00"
              required
            />
          </div>
          <div v-if="form.mode === 'rolling'" class="field">
            <label class="field-label" for="key-window">滚动窗口</label
            ><OrbitSelect
              id="key-window"
              v-model="form.window"
              :options="windowOptions"
            />
          </div>
        </div>
        <p class="field-hint">
          {{
            form.mode === "unlimited"
              ? "不单独限制此密钥，仍受账户可用余额与预算约束。"
              : form.mode === "rolling"
                ? "按所选窗口限制消费，窗口状态以平台记录为准。"
                : "累计消费达到限额后，付费调用将受限。"
          }}
        </p>
      </div>
      <label v-if="!editingId" class="key-create-check"
        ><input v-model="form.use" type="checkbox" /><span
          >创建后设为 Orbit 默认密钥</span
        ></label
      >
      <div class="dialog-actions">
        <button
          type="button"
          class="button secondary"
          @click="editorOpen = false"
        >
          取消</button
        ><button type="submit" class="button primary" :disabled="readOnly">
          {{ editingId ? "保存更改" : form.use ? "创建并使用" : "创建密钥" }}
        </button>
      </div>
    </form></OrbitDialog
  >

  <OrbitDialog
    v-model:open="confirmOpen"
    :title="pendingAction.title"
    :description="pendingAction.description"
    ><div v-if="pendingKey" class="key-confirm-target">
      <OrbitIcon name="key" :size="20" />
      <div>
        <strong>{{ pendingKey.name }}</strong
        ><span class="key-prefix mono">{{ pendingKey.prefix }}</span>
      </div>
    </div>
    <p
      v-if="pending?.action === 'reset' && pendingKey"
      class="key-confirm-note"
    >
      当前已用 {{ money(pendingKey.used) }}，重置后为 $0.00。
    </p>
    <div class="dialog-actions">
      <button class="button secondary" @click="confirmOpen = false">取消</button
      ><button
        class="button"
        :class="pending?.action === 'reset' ? 'primary' : 'danger-button'"
        :disabled="readOnly"
        @click="confirm"
      >
        {{ pendingAction.verb }}
      </button>
    </div></OrbitDialog
  >

  <OrbitDialog
    v-model:open="revealOpen"
    title="查看与复制密钥"
    description="请妥善保管密钥，只在可信任的工具中使用。"
    ><p class="key-secret-label">{{ revealKey?.name }}</p>
    <div class="key-reveal-value mono">
      {{
        revealed
          ? `orv_sk_DEMO_NOT_A_REAL_KEY_${revealKey?.id}_2026`
          : "orv_sk_••••••••••••••••••••••••••••"
      }}
    </div>
    <p class="field-hint">显示后 30 秒自动隐藏；离开窗口时也会隐藏。</p>
    <div class="dialog-actions">
      <button
        class="button secondary"
        @click="revealed ? (revealed = false) : reveal()"
      >
        {{ revealed ? "隐藏密钥" : "显示密钥" }}</button
      ><button class="button primary" @click="copy">
        <OrbitIcon name="copy" :size="15" />复制密钥
      </button>
    </div></OrbitDialog
  >

  <OrbitDialog
    v-model:open="updateOpen"
    :title="`更新 ${updateClient ? clientLabels[updateClient] : ''} 接入`"
    description="将此客户端更新为 Orbit 当前默认密钥。接入前的原配置快照继续保留。"
    ><div class="key-update-preview">
      <div>
        <span>当前绑定</span
        ><strong>{{ updateClient ? keyName(bound[updateClient]) : "" }}</strong>
      </div>
      <OrbitIcon name="arrow" />
      <div>
        <span>更新为</span><strong>{{ current?.name }}</strong>
      </div>
    </div>
    <p class="field-hint">配置保存后，请完整退出客户端并重新打开。</p>
    <div class="dialog-actions">
      <button class="button secondary" @click="updateOpen = false">取消</button
      ><button
        class="button primary"
        :disabled="readOnly || current?.status !== 'active'"
        @click="applyUpdate"
      >
        确认更新
      </button>
    </div></OrbitDialog
  >

  <div v-if="toast" class="toast" role="status">
    <span>{{ toast }}</span
    ><button class="icon-button" aria-label="关闭提示" @click="toast = ''">
      <OrbitIcon name="close" :size="15" />
    </button>
  </div>
</template>
