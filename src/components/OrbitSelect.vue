<script setup lang="ts">
import {
  SelectContent,
  SelectIcon,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectPortal,
  SelectRoot,
  SelectTrigger,
  SelectValue,
  SelectViewport,
} from "reka-ui";
import OrbitIcon from "./OrbitIcon.vue";

defineProps<{
  id: string;
  modelValue: string;
  options: { value: string; label: string; disabled?: boolean }[];
  placeholder?: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();
</script>

<template>
  <SelectRoot
    :model-value="modelValue"
    :disabled="disabled"
    @update:model-value="
      (value) => typeof value === 'string' && emit('update:modelValue', value)
    "
  >
    <SelectTrigger :id="id" class="select-trigger">
      <SelectValue :placeholder="placeholder" class="select-value" />
      <SelectIcon as-child
        ><OrbitIcon name="chevron-down" :size="16"
      /></SelectIcon>
    </SelectTrigger>
    <SelectPortal>
      <SelectContent
        position="popper"
        align="start"
        :side-offset="6"
        :collision-padding="12"
        class="select-content"
      >
        <SelectViewport class="select-viewport">
          <SelectItem
            v-for="option in options"
            :key="option.value"
            :value="option.value"
            :disabled="option.disabled"
            class="select-option"
          >
            <SelectItemText>{{ option.label }}</SelectItemText>
            <SelectItemIndicator as-child
              ><OrbitIcon name="check" :size="16"
            /></SelectItemIndicator>
          </SelectItem>
        </SelectViewport>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>
