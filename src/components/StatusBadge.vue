<script setup lang="ts">
withDefaults(
  defineProps<{
    tone?: "neutral" | "success" | "warning" | "error";
    text?: string;
  }>(),
  { tone: "neutral", text: "" },
);
</script>
<template>
  <span class="badge" :class="`badge-${tone}`"
    ><span class="badge-dot" aria-hidden="true" /><slot>{{ text }}</slot></span
  >
</template>
