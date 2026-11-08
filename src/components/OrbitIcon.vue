<script setup lang="ts">
defineProps<{ name: string; size?: number }>();
const paths: Record<string, string> = {
  home: "M3 10 12 3l9 7v10a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1Z",
  images: "M4 4h16v16H4ZM4 16l5-5 4 4 3-3 4 4M8 8h.01",
  access: "M8 3v5M16 3v5M5 8h14v3a7 7 0 0 1-14 0ZM12 18v3",
  settings:
    "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4",
  arrow: "M5 12h14M13 6l6 6-6 6",
  check: "m5 12 4 4 10-10",
  close: "m6 6 12 12M18 6 6 18",
  plus: "M12 5v14M5 12h14",
  external: "M14 3h7v7M21 3l-9 9M10 3H4v17h17v-6",
  shield: "m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6ZM8 12l3 3 5-6",
  folder: "M3 6h7l2 2h9v12H3Z",
  download: "M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4",
  copy: "M8 8h13v13H8ZM16 8V3H3v13h5",
  trash: "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18ZM12 7v5l3 2",
  spark: "m12 2 3 7 7 3-7 3-3 7-3-7-7-3 7-3Z",
  zoom: "M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14ZM15 15l6 6M7 10h6M10 7v6",
  node: "M5 5h14v6H5ZM5 15h14v6H5ZM8 8h.01M8 18h.01M12 11v4",
  key: "M7 3a5 5 0 1 0 0 10 5 5 0 0 0 0-10ZM11 11l10 10M17 17l3-3",
  chevron: "m9 5 7 7-7 7",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  account:
    "M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM4 21v-3a5 5 0 0 1 5-5h6a5 5 0 0 1 5 5v3",
  refresh:
    "M20 8a9 9 0 0 0-15-3L2 8M2 3v5h5M4 16a9 9 0 0 0 15 3l3-3M22 21v-5h-5",
  file: "M4 3h10l6 6v12H4ZM14 3v6h6M8 13h8M8 17h6",
  history: "M3 10a9 9 0 1 1 2 9M3 4v6h6M12 7v5l4 2",
  upload: "M12 16V3m-5 5 5-5 5 5M4 16v5h16v-5",
  edit: "M14 4l6 6M3 21l2-7L17 2l5 5L10 19ZM3 21l7-2",
  loader: "M12 3a9 9 0 0 1 9 9M12 21a9 9 0 0 1-9-9",
  sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5",
  window: "M3 4h18v16H3ZM3 9h18M6 6.5h.01M9 6.5h.01",
};
</script>
<template>
  <svg
    class="icon"
    :width="size ?? 20"
    :height="size ?? 20"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path :d="paths[name] ?? paths.spark" />
  </svg>
</template>
