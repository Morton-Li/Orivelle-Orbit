<script setup lang="ts">
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import OrbitIcon from "./OrbitIcon.vue";
import { t } from "../shared/i18n";
import { watch } from "vue";
const props = defineProps<{
  open: boolean;
  title: string;
  description?: string;
  wide?: boolean;
}>();
defineEmits<{ "update:open": [value: boolean] }>();
let previousFocus: HTMLElement | null = null;
watch(
  () => props.open,
  (open) => {
    if (open)
      previousFocus =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
  },
  { flush: "sync" },
);
function restoreFocus(event: Event) {
  event.preventDefault();
  if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
  else
    document
      .querySelector<HTMLElement>("#main-content")
      ?.focus({ preventScroll: true });
}
</script>
<template>
  <DialogRoot :open="open" @update:open="$emit('update:open', $event)">
    <DialogPortal
      ><DialogOverlay class="dialog-overlay" /><DialogContent
        class="dialog-content"
        :class="{ 'dialog-wide': wide }"
        @close-auto-focus="restoreFocus"
      >
        <div class="dialog-heading">
          <DialogTitle class="dialog-title">{{ title }}</DialogTitle
          ><DialogClose class="icon-button" :aria-label="t('action.close')"
            ><OrbitIcon name="close"
          /></DialogClose>
        </div>
        <DialogDescription class="muted dialog-description">{{
          description ?? ""
        }}</DialogDescription
        ><slot /> </DialogContent
    ></DialogPortal>
  </DialogRoot>
</template>
