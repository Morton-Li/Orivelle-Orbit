<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { TabsContent, TabsList, TabsRoot, TabsTrigger } from "reka-ui";
import { useOrbitStore } from "../stores/orbit";
import { subscribeImageInputs } from "../shared/bridge";
import type {
  HistoryImage,
  ImageInput,
  ImageParameters,
  TaskView,
} from "../shared/contracts";
import { CAPABILITIES } from "../shared/contracts";
import {
  formatBytes,
  formatUpdatedAt,
  nodeName,
  safePreviewUrl,
  utf8Bytes,
} from "../shared/format";
import { t } from "../shared/i18n";
import OrbitIcon from "../components/OrbitIcon.vue";
import OrbitDialog from "../components/OrbitDialog.vue";
import OrbitSelect from "../components/OrbitSelect.vue";
import StatusBadge from "../components/StatusBadge.vue";
import {
  imageParameterFields,
  availableImageConstraints,
  imageParameterLabel,
  imageParameterError,
  supportedImageParameters,
  imageFailureMessage,
  type ImageSelectParameter,
} from "../shared/image-parameters";
const store = useOrbitStore();
const tab = ref("create");
const prompt = ref("");
const modelId = ref("");
const inputs = ref<ImageInput[]>([]);
const parameters = ref<ImageParameters>({});
const advanced = ref(false);
const activeTaskId = ref<string | null>(null);
const enlarged = ref<HistoryImage | null>(null);
const deleteImageIds = ref<string[]>([]);
const now = ref(Date.now());
let timer: number | undefined;
let unsubscribe: (() => void) | undefined;
let alive = true;
const model = computed(
  () => store.imageModels.find((item) => item.id === modelId.value) ?? null,
);
const constraints = computed(() =>
  availableImageConstraints(model.value?.image_parameters ?? null),
);
const currentNode = computed(() => {
  const index = store.state.nodes.findIndex(
    (node) => node.id === store.activeConnection?.node_id,
  );
  const node = store.state.nodes[index];
  return node
    ? nodeName(node, index)
    : t(store.account ? "access.noNode" : "access.noConnection");
});
const parameterFields = computed(() => imageParameterFields(constraints.value));
function supportedParameters(values: ImageParameters): ImageParameters {
  const supported = supportedImageParameters(values, constraints.value);
  const backgrounds = constraints.value?.backgrounds ?? [];
  const background =
    supported.background ??
    (backgrounds.includes("opaque") ? "opaque" : backgrounds[0]);
  return background ? { ...supported, background } : supported;
}
const currentParameters = computed(() => parameters.value);
const parameterError = computed(() =>
  model.value ? imageParameterError(parameters.value, constraints.value) : null,
);
const promptBytes = computed(() => utf8Bytes(prompt.value));
const promptLimit = computed(() =>
  Math.min(constraints.value?.prompt_maximum_bytes ?? 32768, 32768),
);
const activeTask = computed(
  () =>
    store.tasks.find((task) => task.id === activeTaskId.value) ??
    store.tasks.find(
      (task) =>
        task.connection_id === store.activeConnection?.id &&
        task.connection_revision === store.activeConnection.revision,
    ) ??
    null,
);
const requestedModelLabel = computed(
  () =>
    activeTask.value?.requested_model_label ??
    store.models.find((item) => item.id === activeTask.value?.requested_model)
      ?.label ??
    null,
);
const resultImages = computed(() =>
  store.history.filter((image) =>
    activeTask.value?.image_ids.includes(image.id),
  ),
);
const active = computed(
  () =>
    !!activeTask.value &&
    ["queued", "sending", "waiting", "saving"].includes(activeTask.value.state),
);
const canSubmit = computed(
  () =>
    store.ready &&
    !!model.value &&
    prompt.value.trim().length > 0 &&
    promptBytes.value <= promptLimit.value &&
    !parameterError.value &&
    inputs.value.length <= (constraints.value?.max_inputs ?? 5) &&
    !active.value &&
    !store.busy.has("image-submit") &&
    model.value.capabilities.includes(
      inputs.value.length
        ? CAPABILITIES.imageEdit
        : CAPABILITIES.imageGeneration,
    ),
);
const elapsed = computed(() =>
  activeTask.value?.started_at
    ? Math.max(
        0,
        Math.floor(
          (now.value - Date.parse(activeTask.value.started_at)) / 1000,
        ),
      )
    : 0,
);
function taskLabel(task: TaskView) {
  return t(
    task.state === "save_failed"
      ? "images.saveFailed"
      : task.state === "completed"
        ? "images.completed"
        : task.state === "failed"
          ? "images.failed"
          : task.state === "cancelled"
            ? "images.cancelled"
            : task.state === "unconfirmed"
              ? "images.unconfirmed"
              : task.state === "saving"
                ? "images.saving"
                : "images.waiting",
  );
}
function taskTone(task: TaskView) {
  return task.state === "completed"
    ? ("success" as const)
    : task.state === "failed" || task.state === "save_failed"
      ? ("error" as const)
      : task.state === "unconfirmed" || task.state === "cancelled"
        ? ("warning" as const)
        : ("neutral" as const);
}
watch(
  () => store.scope,
  () => {
    prompt.value = "";
    inputs.value = [];
    modelId.value = "";
    parameters.value = {};
    advanced.value = false;
    activeTaskId.value = null;
    enlarged.value = null;
    deleteImageIds.value = [];
  },
);
watch(modelId, () => {
  parameters.value = supportedParameters({});
  advanced.value = false;
});
watch(constraints, () => {
  parameters.value = supportedParameters(parameters.value);
});
watch(
  () => store.imageModels,
  () => {
    if (!store.imageModels.some((item) => item.id === modelId.value))
      modelId.value = "";
  },
);
onMounted(async () => {
  timer = window.setInterval(() => {
    now.value = Date.now();
  }, 1000);
  const stop = await subscribeImageInputs((event) => {
    const connection = store.activeConnection;
    if (
      alive &&
      connection &&
      event.account_id === connection.account_id &&
      event.connection_id === connection.id &&
      event.connection_revision === connection.revision
    )
      addInputs(event.inputs);
  });
  if (alive) unsubscribe = stop;
  else stop();
});
onUnmounted(() => {
  alive = false;
  if (timer) window.clearInterval(timer);
  unsubscribe?.();
});
function addInputs(values: ImageInput[]) {
  const limit = constraints.value?.max_inputs ?? 5;
  const seen = new Set(inputs.value.map((item) => item.id));
  const added = values.filter((item) => !seen.has(item.id));
  if (inputs.value.length + added.length > limit) {
    store.notify(t("images.inputLimit"), "error");
    return;
  }
  inputs.value.push(...added);
}
async function chooseInputs() {
  const connection = store.activeConnection;
  const captured = store.scope;
  if (!connection) return;
  const result = await store.perform("image-inputs", "select_image_inputs", {
    connection_id: connection.id,
  });
  if (result && captured === store.scope) addInputs(result);
}
async function submit() {
  const connection = store.activeConnection;
  const captured = store.scope;
  if (!connection || !canSubmit.value) return;
  const result = await store.perform("image-submit", "create_image_task", {
    connection_id: connection.id,
    expected_revision: connection.revision,
    model_id: modelId.value,
    prompt: prompt.value,
    parameters: { ...currentParameters.value },
    input_ids: inputs.value.map((input) => input.id),
  });
  if (result && captured === store.scope) {
    store.acceptTask(result);
    activeTaskId.value = result.id;
  }
  if (result) await store.refresh();
}
async function cancel() {
  if (!activeTask.value) return;
  const result = await store.perform("image-cancel", "cancel_image_task", {
    task_id: activeTask.value.id,
  });
  if (result) {
    store.acceptTask(result);
    await store.refresh();
  }
}
async function retrySave() {
  if (
    !activeTask.value ||
    !activeTask.value.local_save_available ||
    !["save_failed", "unconfirmed"].includes(activeTask.value.state)
  )
    return;
  const result = await store.perform("image-save", "retry_local_save", {
    task_id: activeTask.value.id,
  });
  if (result) {
    store.acceptTask(result);
    await store.refresh();
  }
}
async function save(image: HistoryImage) {
  const result = await store.perform(`save-${image.id}`, "save_image_as", {
    image_id: image.id,
  });
  if (result?.saved) store.notify(t("common.saved"), "success");
}
async function copy(image: HistoryImage) {
  const result = await store.perform(`copy-${image.id}`, "copy_image", {
    image_id: image.id,
  });
  if (result?.copied) store.notify(t("common.copied"), "success");
}
async function edit(image: HistoryImage) {
  const connection = store.activeConnection;
  const captured = store.scope;
  if (!connection || !store.ready) return;
  const result = await store.perform("image-reedit", "use_history_as_input", {
    image_id: image.id,
    connection_id: connection.id,
    expected_revision: connection.revision,
  });
  if (result && captured === store.scope) {
    inputs.value = [result];
    if (store.imageModels.some((item) => item.id === image.requested_model))
      modelId.value = image.requested_model;
    activeTaskId.value = null;
    tab.value = "create";
    enlarged.value = null;
  }
}
async function deleteHistory() {
  const result = await store.perform("history-delete", "delete_history", {
    image_ids: [...deleteImageIds.value],
    confirmed: true,
  });
  if (result) {
    deleteImageIds.value = [];
    enlarged.value = null;
    await store.refresh();
    store.notify(t("common.deleted"), "success");
  }
}
function setParameter(key: ImageSelectParameter, value: string) {
  parameters.value = {
    ...parameters.value,
    [key]: key === "n" ? Number(value) : value,
  };
}
function imageDescription(image: HistoryImage) {
  return (
    store.tasks.find(
      (task) =>
        task.id === image.task_id && task.account_id === image.account_id,
    )?.prompt ?? t("images.generate")
  );
}
</script>
<template>
  <section class="page-heading">
    <p class="eyebrow">{{ t("images.eyebrow") }}</p>
    <h1>{{ t("images.title") }}</h1>
    <p class="page-description">{{ t("images.description") }}</p>
  </section>
  <TabsRoot v-model="tab" class="image-tabs"
    ><TabsList class="tab-list" :aria-label="t('nav.images')"
      ><TabsTrigger value="create" class="tab-trigger"
        ><OrbitIcon name="images" />{{ t("images.create") }}</TabsTrigger
      ><TabsTrigger value="history" class="tab-trigger"
        ><OrbitIcon name="history" />{{ t("images.history")
        }}<span v-if="store.history.length" class="tab-count">{{
          store.history.length
        }}</span></TabsTrigger
      ></TabsList
    >
    <TabsContent value="create" class="image-workbench"
      ><form class="image-composer" @submit.prevent="submit">
        <div class="composer-connection">
          <span
            class="connection-dot"
            :class="{ connected: store.ready }"
          /><span class="truncate">{{ currentNode }}</span
          ><RouterLink to="/access" class="text-button">{{
            t("action.manage")
          }}</RouterLink>
        </div>
        <div class="field">
          <label for="image-model" class="field-label">{{
            t("access.model")
          }}</label
          ><OrbitSelect
            id="image-model"
            v-model="modelId"
            :options="
              store.imageModels.map((item) => ({
                value: item.id,
                label: item.label,
              }))
            "
            :placeholder="t('access.selectModel')"
            :disabled="
              store.modelsLoading || active || !store.imageModels.length
            "
          />
        </div>
        <div class="field prompt-field">
          <label for="image-prompt" class="field-label">{{
            t("images.prompt")
          }}</label
          ><textarea
            id="image-prompt"
            v-model="prompt"
            :placeholder="t('images.promptPlaceholder')"
            :aria-invalid="promptBytes > promptLimit"
            :disabled="active"
            rows="7"
          />
          <div
            class="prompt-count"
            :class="{ 'error-text': promptBytes > promptLimit }"
          >
            {{ promptBytes.toLocaleString() }} /
            {{ promptLimit.toLocaleString() }} UTF-8 B
          </div>
          <p v-if="promptBytes > promptLimit" class="error-text" role="alert">
            {{ t("images.promptLimit") }}
          </p>
        </div>
        <div class="field">
          <div class="field-heading">
            <span class="field-label">{{ t("images.inputs") }}</span
            ><button
              type="button"
              class="text-button"
              :disabled="
                !store.ready || active || store.busy.has('image-inputs')
              "
              @click="chooseInputs"
            >
              <OrbitIcon name="plus" />{{ t("images.addInputs") }}
            </button>
          </div>
          <button
            v-if="!inputs.length"
            type="button"
            class="image-drop-area"
            :disabled="!store.ready || active"
            @click="chooseInputs"
          >
            <OrbitIcon name="upload" /><span>{{ t("images.drop") }}</span>
          </button>
          <ul v-else class="input-images">
            <li v-for="input in inputs" :key="input.id">
              <img
                v-if="safePreviewUrl(input.preview_url)"
                :src="safePreviewUrl(input.preview_url)!"
                :alt="input.name"
              /><OrbitIcon v-else name="images" /><span class="truncate"
                >{{ input.name
                }}<small
                  >{{ input.width }} × {{ input.height }} ·
                  {{ formatBytes(input.bytes) }}</small
                ></span
              ><button
                type="button"
                class="icon-button"
                :disabled="active"
                :aria-label="`${t('images.removeInput')}: ${input.name}`"
                @click="inputs = inputs.filter((item) => item.id !== input.id)"
              >
                <OrbitIcon name="close" />
              </button>
            </li>
          </ul>
        </div>
        <section v-if="constraints" class="parameter-details">
          <button
            type="button"
            class="advanced-trigger"
            :aria-expanded="advanced"
            aria-controls="image-parameters"
            :disabled="active"
            @click="advanced = !advanced"
          >
            <OrbitIcon name="settings" :size="17" />
            {{ t("images.params") }}
            <OrbitIcon name="chevron-down" :size="16" />
          </button>
          <div v-if="advanced" id="image-parameters">
            <div v-if="parameterFields.length" class="parameter-grid">
              <div
                v-for="field in parameterFields"
                :key="field.key"
                class="field"
              >
                <label :for="`image-${field.key}`" class="field-label">{{
                  t(field.label)
                }}</label>
                <OrbitSelect
                  :id="`image-${field.key}`"
                  :model-value="String(currentParameters[field.key] ?? '')"
                  :disabled="active"
                  :options="[
                    ...field.values.map((value) => ({
                      value,
                      label: imageParameterLabel(value),
                    })),
                  ]"
                  @update:model-value="setParameter(field.key, $event)"
                />
              </div>
            </div>
            <p v-else class="field-hint">{{ t("images.noAdvancedOptions") }}</p>
            <p
              v-if="parameterFields.some((field) => field.key === 'background')"
              class="field-hint"
            >
              {{ t("images.backgroundUsageHint") }}
            </p>
          </div>
          <p v-if="parameterError" class="error-text" role="alert">
            {{ t(parameterError) }}
          </p>
        </section>
        <button
          type="submit"
          class="button primary generate-button"
          :disabled="!canSubmit"
        >
          <OrbitIcon name="spark" />{{
            t(inputs.length ? "images.edit" : "images.generate")
          }}
        </button>
      </form>
      <section class="image-result" :aria-label="t('images.empty')">
        <template v-if="activeTask"
          ><div class="task-header">
            <StatusBadge
              :text="taskLabel(activeTask)"
              :tone="taskTone(activeTask)"
            /><span v-if="active" class="elapsed" role="timer"
              >{{ elapsed }} {{ t("common.seconds") }}</span
            >
          </div>
          <p v-if="activeTask.error" class="error-text" role="alert">
            {{ imageFailureMessage(activeTask.error.code) }}
          </p>
          <div v-if="resultImages.length" class="result-grid">
            <article
              v-for="image in resultImages"
              :key="image.id"
              class="result-card"
            >
              <button
                class="image-preview-button"
                :style="{ aspectRatio: `${image.width} / ${image.height}` }"
                :disabled="!safePreviewUrl(image.preview_url)"
                :aria-label="t('images.zoom')"
                @click="enlarged = image"
              >
                <img
                  v-if="safePreviewUrl(image.preview_url)"
                  :src="safePreviewUrl(image.preview_url)!"
                  :alt="activeTask.prompt"
                  :width="image.width"
                  :height="image.height"
                /><OrbitIcon v-else name="images" />
              </button>
              <div class="image-card-meta">
                <span>{{ image.width }} × {{ image.height }}</span
                ><span>{{
                  image.mime.replace("image/", "").toUpperCase()
                }}</span
                ><span>{{ formatBytes(image.bytes) }}</span>
              </div>
              <div class="image-card-actions">
                <button
                  class="icon-button"
                  :disabled="store.busy.has(`save-${image.id}`)"
                  :aria-label="t('action.save')"
                  @click="save(image)"
                >
                  <OrbitIcon name="download" /></button
                ><button
                  class="icon-button"
                  :disabled="store.busy.has(`copy-${image.id}`)"
                  :aria-label="t('action.copy')"
                  @click="copy(image)"
                >
                  <OrbitIcon name="copy" /></button
                ><button
                  class="icon-button"
                  :disabled="!store.ready || store.busy.has('image-reedit')"
                  :aria-label="t('action.edit')"
                  @click="edit(image)"
                >
                  <OrbitIcon name="edit" />
                </button>
              </div>
            </article>
          </div>
          <div v-else class="task-waiting">
            <OrbitIcon
              :name="active ? 'loader' : 'images'"
              :class="{ spinning: active }"
            />
            <p>{{ taskLabel(activeTask) }}</p>
            <p
              v-if="
                activeTask.state === 'unconfirmed' ||
                activeTask.state === 'cancelled'
              "
              class="muted"
            >
              {{ t("images.cancelHint") }}
            </p>
          </div>
          <div
            v-if="
              (active && activeTask.state !== 'saving') ||
              (activeTask.local_save_available &&
                ['save_failed', 'unconfirmed'].includes(activeTask.state))
            "
            class="task-actions"
          >
            <button
              v-if="active && activeTask.state !== 'saving'"
              class="button secondary"
              :disabled="store.busy.has('image-cancel')"
              @click="cancel"
            >
              {{ t("action.cancel") }}</button
            ><button
              v-if="
                activeTask.local_save_available &&
                ['save_failed', 'unconfirmed'].includes(activeTask.state)
              "
              class="button secondary"
              :disabled="store.busy.has('image-save')"
              @click="retrySave"
            >
              {{ t("action.retrySave") }}
            </button>
          </div>
          <dl
            v-if="requestedModelLabel || activeTask.request_id"
            class="task-facts"
          >
            <div v-if="requestedModelLabel">
              <dt>{{ t("images.requestedModel") }}</dt>
              <dd>{{ requestedModelLabel }}</dd>
            </div>
            <div v-if="activeTask.request_id">
              <dt>Request ID</dt>
              <dd class="mono">{{ activeTask.request_id }}</dd>
            </div>
          </dl></template
        >
        <div v-else class="image-empty">
          <span class="empty-art" aria-hidden="true"
            ><OrbitIcon name="images"
          /></span>
          <h2>{{ t("images.empty") }}</h2>
        </div>
      </section></TabsContent
    >
    <TabsContent value="history" class="history-content"
      ><div v-if="!store.history.length" class="empty-state">
        <OrbitIcon name="history" />
        <h2>{{ t("images.noHistory") }}</h2>
        <p class="muted">{{ t("images.noHistoryHint") }}</p>
      </div>
      <div v-else class="history-grid">
        <article
          v-for="image in store.history"
          :key="image.id"
          class="history-card"
        >
          <button
            class="image-preview-button"
            :disabled="!safePreviewUrl(image.preview_url)"
            :aria-label="t('images.zoom')"
            @click="enlarged = image"
          >
            <img
              v-if="safePreviewUrl(image.preview_url)"
              :src="safePreviewUrl(image.preview_url)!"
              :alt="imageDescription(image)"
              :width="image.width"
              :height="image.height"
              loading="lazy"
            /><OrbitIcon v-else name="images" />
          </button>
          <div class="history-card-info">
            <strong>{{ image.width }} × {{ image.height }}</strong>
            <p class="muted">
              {{ image.mime.replace("image/", "").toUpperCase() }} ·
              {{ formatBytes(image.bytes) }}
            </p>
            <p class="muted">
              <time :datetime="image.created_at">{{
                formatUpdatedAt(image.created_at)
              }}</time>
            </p>
            <div class="image-card-actions">
              <button
                class="icon-button"
                :aria-label="t('action.save')"
                @click="save(image)"
              >
                <OrbitIcon name="download" /></button
              ><button
                class="icon-button"
                :aria-label="t('action.copy')"
                @click="copy(image)"
              >
                <OrbitIcon name="copy" /></button
              ><button
                class="icon-button"
                :disabled="!store.ready"
                :aria-label="t('action.edit')"
                @click="edit(image)"
              >
                <OrbitIcon name="edit" /></button
              ><button
                class="icon-button danger"
                :aria-label="t('action.delete')"
                @click="deleteImageIds = [image.id]"
              >
                <OrbitIcon name="trash" />
              </button>
            </div>
          </div>
        </article></div
    ></TabsContent>
  </TabsRoot>
  <OrbitDialog
    :open="enlarged !== null"
    :title="t('images.zoom')"
    wide
    @update:open="
      (value) => {
        if (!value) enlarged = null;
      }
    "
    ><template v-if="enlarged"
      ><img
        v-if="safePreviewUrl(enlarged.preview_url)"
        class="zoom-image"
        :src="safePreviewUrl(enlarged.preview_url)!"
        :alt="imageDescription(enlarged)"
        :width="enlarged.width"
        :height="enlarged.height"
      />
      <p class="muted">
        {{ enlarged.width }} × {{ enlarged.height }} · {{ enlarged.mime }} ·
        {{ formatBytes(enlarged.bytes) }}
      </p>
      <div class="dialog-actions">
        <button class="button secondary" @click="save(enlarged)">
          {{ t("action.save") }}</button
        ><button class="button secondary" @click="copy(enlarged)">
          {{ t("action.copy") }}</button
        ><button
          class="button primary"
          :disabled="!store.ready"
          @click="edit(enlarged)"
        >
          {{ t("action.edit") }}
        </button>
      </div></template
    ></OrbitDialog
  >
  <OrbitDialog
    :open="deleteImageIds.length > 0"
    :title="t('action.delete')"
    :description="t('images.deleteHint')"
    @update:open="
      (value) => {
        if (!value) deleteImageIds = [];
      }
    "
    ><div class="dialog-actions">
      <button class="button secondary" @click="deleteImageIds = []">
        {{ t("action.cancel") }}</button
      ><button
        class="button danger-button"
        :disabled="store.busy.has('history-delete')"
        @click="deleteHistory"
      >
        {{ t("common.confirm") }}
      </button>
    </div></OrbitDialog
  >
</template>
