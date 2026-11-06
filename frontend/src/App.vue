<script setup lang="ts">
import { defineAsyncComponent } from "vue";
import ToastHost from "./components/ToastHost.vue";
import { useCoverScan } from "./stores/coverScan";

const scan = useCoverScan();

// The scanner (and its detector and warp) loads only when a cover photo is
// chosen, not with every page. A chunk that fails to load hands the photo
// back as it is, as the scanner does with a photo it cannot open.
const CoverScanner = defineAsyncComponent(() =>
  import("./components/CoverScanner.vue").catch((error: unknown) => {
    scan.finish(scan.file);
    throw error;
  }),
);
</script>

<template>
  <RouterView />
  <CoverScanner v-if="scan.file" />
  <ToastHost />
</template>
