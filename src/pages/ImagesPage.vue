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
  imageParameterLabel,
  supportedImageParameters,
  imageFailureMessage,
  type ImageSelectParameter,
} from "../shared/image-parameters";
import {
  workbenchConstraints,
  workbenchParameterError,
  adjustedImageParameters,
} from "../shared/image-api";
const store = useOrbitStore();
const tab = ref("create");
const prompt = ref("");
const modelId = ref("");
const inputs = ref<ImageInput[]>([]);
const mask = ref<ImageInput | null>(null);
const markup = ref<ImageInput | null>(null);
const parameters = ref<ImageParameters>({});
const advanced = ref(false);
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
  workbenchConstraints(
    model.value?.image_parameters ?? null,
    model.value?.image_api_parameters,
  ),
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
const apiCaps = computed(() => model.value?.image_api_parameters ?? null);
const parameterFields = computed(() => {
  const fields = imageParameterFields(constraints.value);
  if (apiCaps.value?.moderations.length)
    fields.push({
      key: "moderation",
      label: "images.moderation",
      values: apiCaps.value.moderations,
    });
  if (inputs.value.length && apiCaps.value?.input_fidelities.length)
    fields.push({
      key: "input_fidelity",
      label: "images.inputFidelity",
      values: apiCaps.value.input_fidelities,
    });
  return fields;
});
const usesApi = computed(
  () =>
    adjustedImageParameters(
      parameters.value,
      inputs.value.length,
      prompt.value,
      !!mask.value,
      !!markup.value,
    ).length > 0,
);
const customSize = computed(
  () =>
    !!parameters.value.size &&
    parameters.value.size !== "auto" &&
    !constraints.value?.sizes.includes(parameters.value.size),
);
function fieldValue(key: ImageSelectParameter): string {
  const value = parameters.value[key];
  if (key === "size" && customSize.value) return "__custom__";
  return value === undefined
    ? key === "n"
      ? "1"
      : key === "output_format"
        ? "png"
        : key === "input_fidelity"
          ? "low"
          : key === "complexity"
            ? "__default__"
            : "auto"
    : String(value);
}
function fieldOptions(field: ReturnType<typeof imageParameterFields>[number]) {
  const values = field.values.map((value) => ({
    value,
    label: imageParameterLabel(value),
  }));
  if (field.key === "size" && constraints.value?.custom_size)
    values.push({ value: "__custom__", label: t("images.customSize") });
  if (field.key === "complexity")
    values.unshift({ value: "__default__", label: t("images.serviceDefault") });
  return values;
}
function setDimension(axis: number, event: Event) {
  const size = (parameters.value.size ?? "1536x864").split("x");
  size[axis] = (event.target as HTMLInputElement).value;
  parameters.value = { ...parameters.value, size: size.join("x") };
}
function setNumeric(
  key:
    | "output_compression"
    | "partial_images"
    | "frame_count"
    | "frame_duration_ms",
  event: Event,
) {
  const value = (event.target as HTMLInputElement).value;
  if (!value) {
    const next = { ...parameters.value };
    delete next[key];
    parameters.value = next;
    return;
  }
  parameters.value = { ...parameters.value, [key]: Number(value) };
}
function setStream(event: Event) {
  const stream = (event.target as HTMLInputElement).checked;
  parameters.value = {
    ...parameters.value,
    stream,
    partial_images: stream ? (parameters.value.partial_images ?? 1) : 0,
  };
}
function setSvgModel(event: Event) {
  const value = (event.target as HTMLInputElement).value;
  const next = { ...parameters.value };
  if (!value) delete next.svg;
  else next.svg = { ...next.svg, model: value };
  parameters.value = next;
}
function setSvgEffort(value: string) {
  const next = { ...parameters.value };
  if (value === "__default__") {
    if (next.svg) next.svg = { model: next.svg.model };
  } else
    next.svg = { model: next.svg?.model ?? "", reasoning: { effort: value } };
  parameters.value = next;
}
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
  model.value
    ? workbenchParameterError(
        parameters.value,
        constraints.value,
        apiCaps.value,
        inputs.value.length,
        !!mask.value,
        !!markup.value,
      )
    : null,
);
const promptBytes = computed(() =>
  apiCaps.value ? [...prompt.value].length : utf8Bytes(prompt.value),
);
const promptLimit = computed(() =>
  apiCaps.value
    ? Math.min(32000, apiCaps.value.prompt_maximum_characters)
    : Math.min(constraints.value?.prompt_maximum_bytes ?? 32768, 32768),
);
const activeTask = computed(() => store.imagePreviewTask);
const requestedModelLabel = computed(
  () =>
    activeTask.value?.requested_model_label ??
    store.models.find((item) => item.id === activeTask.value?.requested_model)
      ?.label ??
    null,
);
const resultImages = computed(() =>
  store.history.filter(
    (image) =>
      activeTask.value?.id === store.imagePreviewTaskId &&
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
    mask.value = null;
    markup.value = null;
    modelId.value = "";
    parameters.value = {};
    advanced.value = false;
    enlarged.value = null;
    deleteImageIds.value = [];
  },
);
watch(modelId, () => {
  mask.value = null;
  markup.value = null;
  parameters.value = supportedParameters({});
  advanced.value = false;
});
watch(
  () => inputs.value[0]?.id,
  () => {
    mask.value = null;
    markup.value = null;
    if (!inputs.value.length) {
      const next = { ...parameters.value };
      delete next.input_fidelity;
      parameters.value = next;
    }
  },
);
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
async function chooseMaterial(kind: "mask" | "markup") {
  const connection = store.activeConnection;
  const captured = store.scope;
  if (!connection) return;
  const result = await store.perform(`image-${kind}`, "select_image_inputs", {
    connection_id: connection.id,
    purpose: kind,
  });
  if (!result?.length || captured !== store.scope) return;
  const item = result[0]!;
  const first = inputs.value[0];
  if (
    result.length !== 1 ||
    item.mime !== "image/png" ||
    !first ||
    item.width !== first.width ||
    item.height !== first.height
  ) {
    store.notify(t("images.invalidMaterial"), "error");
    return;
  }
  if (kind === "mask") {
    mask.value = item;
    markup.value = null;
  } else {
    markup.value = item;
    mask.value = null;
  }
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
    ...(mask.value ? { mask_input_id: mask.value.id } : {}),
    ...(markup.value ? { markup_input_id: markup.value.id } : {}),
  });
  if (result && captured === store.scope) {
    store.acceptTask(result);
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
  if (!referenceAllowed(image)) {
    store.notify(t("images.rasterReferenceOnly"), "error");
    return;
  }
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
    store.imagePreviewTaskId = null;
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
  const next = { ...parameters.value };
  if (value === "__default__") {
    delete next[key];
    parameters.value = next;
    return;
  }
  if (key === "size" && value === "__custom__") {
    parameters.value = { ...next, size: "1536x864" };
    return;
  }
  if (key === "output_format") {
    if (!["jpeg", "webp"].includes(value)) delete next.output_compression;
    if (value !== "svg") delete next.svg;
    if (value !== "gif") {
      delete next.frame_count;
      delete next.frame_duration_ms;
    }
    if (["svg", "gif"].includes(value)) {
      delete next.stream;
      delete next.partial_images;
    }
  }
  parameters.value = { ...next, [key]: key === "n" ? Number(value) : value };
}
function imageDescription(image: HistoryImage) {
  return (
    store.tasks.find(
      (task) =>
        task.id === image.task_id && task.account_id === image.account_id,
    )?.prompt ?? t("images.generate")
  );
}
function imageFormat(image: HistoryImage) {
  return image.mime === "image/svg+xml"
    ? "SVG"
    : image.mime.replace("image/", "").toUpperCase();
}
function referenceAllowed(image: HistoryImage) {
  return ["image/png", "image/jpeg", "image/webp"].includes(image.mime);
}
function copyLabel(image: HistoryImage) {
  return t(
    image.mime === "image/gif" ? "images.copyFirstFrame" : "action.copy",
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
            {{ promptLimit.toLocaleString() }}
            {{ apiCaps ? t("images.characters") : "UTF-8 B" }}
          </div>
          <p v-if="promptBytes > promptLimit" class="error-text" role="alert">
            {{
              t(apiCaps ? "images.promptCharacterLimit" : "images.promptLimit")
            }}
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
                  :model-value="fieldValue(field.key)"
                  :disabled="active"
                  :options="fieldOptions(field)"
                  @update:model-value="setParameter(field.key, $event)"
                />
              </div>
            </div>
            <p v-else class="field-hint">{{ t("images.noAdvancedOptions") }}</p>
            <p v-if="!apiCaps" class="field-hint">
              {{ t("images.apiUpgradeHint") }}
            </p>
            <div v-if="customSize" class="parameter-grid">
              <div class="field">
                <label for="image-custom-width" class="field-label">{{
                  t("images.width")
                }}</label
                ><input
                  id="image-custom-width"
                  type="number"
                  min="16"
                  max="3840"
                  step="16"
                  :value="parameters.size?.split('x')[0]"
                  :disabled="active"
                  @input="setDimension(0, $event)"
                />
              </div>
              <div class="field">
                <label for="image-custom-height" class="field-label">{{
                  t("images.height")
                }}</label
                ><input
                  id="image-custom-height"
                  type="number"
                  min="16"
                  max="3840"
                  step="16"
                  :value="parameters.size?.split('x')[1]"
                  :disabled="active"
                  @input="setDimension(1, $event)"
                />
              </div>
            </div>
            <div
              v-if="
                apiCaps &&
                ['jpeg', 'webp'].includes(parameters.output_format ?? '')
              "
              class="field"
            >
              <label for="image-compression" class="field-label">{{
                t("images.compression")
              }}</label
              ><input
                id="image-compression"
                type="number"
                min="0"
                max="100"
                :value="parameters.output_compression ?? 100"
                :disabled="active"
                @input="setNumeric('output_compression', $event)"
              />
            </div>
            <div
              v-if="apiCaps && parameters.output_format === 'gif'"
              class="parameter-grid"
            >
              <div class="field">
                <label for="image-frame-count" class="field-label">{{
                  t("images.frameCount")
                }}</label
                ><input
                  id="image-frame-count"
                  type="number"
                  min="2"
                  max="16"
                  :value="parameters.frame_count ?? 9"
                  :disabled="active"
                  @input="setNumeric('frame_count', $event)"
                />
              </div>
              <div class="field">
                <label for="image-frame-duration" class="field-label">{{
                  t("images.frameDuration")
                }}</label
                ><input
                  id="image-frame-duration"
                  type="number"
                  min="20"
                  max="655350"
                  step="10"
                  :value="parameters.frame_duration_ms ?? 200"
                  :disabled="active"
                  @input="setNumeric('frame_duration_ms', $event)"
                />
              </div>
            </div>
            <div
              v-if="apiCaps && parameters.output_format === 'svg'"
              class="field"
            >
              <label for="image-svg-model" class="field-label">{{
                t("images.svgAuthor")
              }}</label
              ><input
                id="image-svg-model"
                :value="parameters.svg?.model ?? ''"
                :placeholder="t('images.svgAuthorPlaceholder')"
                :disabled="active"
                @input="setSvgModel"
              />
              <label for="image-svg-reasoning" class="field-label">{{
                t("images.svgReasoning")
              }}</label
              ><OrbitSelect
                id="image-svg-reasoning"
                :model-value="
                  parameters.svg?.reasoning?.effort ?? '__default__'
                "
                :options="[
                  { value: '__default__', label: t('images.serviceDefault') },
                  ...apiCaps.svg_reasoning_efforts.map((value) => ({
                    value,
                    label: imageParameterLabel(value),
                  })),
                ]"
                :disabled="active"
                @update:model-value="setSvgEffort"
              />
            </div>
            <div
              v-if="
                apiCaps?.stream &&
                !['svg', 'gif'].includes(parameters.output_format ?? '')
              "
              class="field"
            >
              <label class="image-stream-toggle"
                ><input
                  id="image-stream"
                  type="checkbox"
                  :checked="parameters.stream ?? false"
                  :disabled="active"
                  @change="setStream"
                />{{ t("images.stream") }}</label
              >
              <p class="field-hint">{{ t("images.streamHint") }}</p>
              <template v-if="parameters.stream"
                ><label for="image-partials" class="field-label">{{
                  t("images.partialImages")
                }}</label
                ><input
                  id="image-partials"
                  type="number"
                  min="0"
                  max="3"
                  :value="parameters.partial_images ?? 0"
                  :disabled="active"
                  @input="setNumeric('partial_images', $event)"
              /></template>
            </div>
            <div
              v-if="
                apiCaps && inputs.length && (apiCaps.mask || apiCaps.markup)
              "
              class="field image-materials"
            >
              <div class="field-heading">
                <button
                  v-if="apiCaps.mask && parameters.output_format !== 'gif'"
                  type="button"
                  class="button secondary"
                  :disabled="active || store.busy.has('image-mask')"
                  @click="chooseMaterial('mask')"
                >
                  {{ t("images.addMask") }}</button
                ><button
                  v-if="apiCaps.markup"
                  type="button"
                  class="button secondary"
                  :disabled="active || store.busy.has('image-markup')"
                  @click="chooseMaterial('markup')"
                >
                  {{ t("images.addMarkup") }}
                </button>
              </div>
              <p class="field-hint">{{ t("images.materialHint") }}</p>
              <div
                v-for="item in [mask, markup].filter(
                  (entry): entry is ImageInput => !!entry,
                )"
                :key="item.id"
                class="image-input-item"
              >
                <span class="truncate"
                  >{{ mask ? t("images.mask") : t("images.markup") }} ·
                  {{ item.name }}</span
                ><button
                  type="button"
                  class="icon-button"
                  :aria-label="t('action.delete')"
                  :disabled="active"
                  @click="
                    mask = null;
                    markup = null;
                  "
                >
                  <OrbitIcon name="close" />
                </button>
              </div>
            </div>
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
        <p
          v-if="model"
          class="image-route-hint"
          :class="{ 'image-route-api': usesApi }"
          role="status"
        >
          {{ t(usesApi ? "images.apiRoute" : "images.subscriptionRoute") }}
        </p>
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
            {{
              imageFailureMessage(
                activeTask.error.code,
                activeTask.error.unsupported_parameters,
              )
            }}
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
                ><span>{{ imageFormat(image) }}</span
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
                  :aria-label="copyLabel(image)"
                  @click="copy(image)"
                >
                  <OrbitIcon name="copy" /></button
                ><button
                  class="icon-button"
                  :disabled="
                    !store.ready ||
                    !referenceAllowed(image) ||
                    store.busy.has('image-reedit')
                  "
                  :title="
                    !referenceAllowed(image)
                      ? t('images.rasterReferenceOnly')
                      : undefined
                  "
                  :aria-label="t('action.edit')"
                  @click="edit(image)"
                >
                  <OrbitIcon name="edit" />
                </button>
              </div>
            </article>
          </div>
          <figure
            v-else-if="
              active &&
              activeTask.partial_preview &&
              safePreviewUrl(activeTask.partial_preview.preview_url)
            "
            class="partial-image-preview"
          >
            <img
              :src="safePreviewUrl(activeTask.partial_preview.preview_url)!"
              :alt="t('images.partialPreview')"
            />
            <figcaption class="field-hint">
              {{ t("images.partialPreview") }}
            </figcaption>
          </figure>
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
              {{ imageFormat(image) }} ·
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
                :aria-label="copyLabel(image)"
                @click="copy(image)"
              >
                <OrbitIcon name="copy" /></button
              ><button
                class="icon-button"
                :disabled="!store.ready || !referenceAllowed(image)"
                :title="
                  !referenceAllowed(image)
                    ? t('images.rasterReferenceOnly')
                    : undefined
                "
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
        {{ enlarged.width }} × {{ enlarged.height }} ·
        {{ imageFormat(enlarged) }} ·
        {{ formatBytes(enlarged.bytes) }}
      </p>
      <p v-if="enlarged.mime === 'image/gif'" class="field-hint">
        {{ t("images.gifPreviewHint") }}
      </p>
      <div class="dialog-actions">
        <button class="button secondary" @click="save(enlarged)">
          {{ t("action.save") }}</button
        ><button class="button secondary" @click="copy(enlarged)">
          {{ copyLabel(enlarged) }}</button
        ><button
          class="button primary"
          :disabled="!store.ready || !referenceAllowed(enlarged)"
          :title="
            !referenceAllowed(enlarged)
              ? t('images.rasterReferenceOnly')
              : undefined
          "
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
