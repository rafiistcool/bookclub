<script setup lang="ts">
import { defineAsyncComponent } from "vue";
import ToastHost from "./components/ToastHost.vue";
import { useCoverScan } from "./stores/coverScan";
import { useFinish } from "./stores/finish";

const scan = useCoverScan();
const finish = useFinish();

// The scanner (and its detector and warp) loads only when a cover photo is
// chosen, not with every page. A chunk that fails to load hands the photo
// back as it is, as the scanner does with a photo it cannot open.
const CoverScanner = defineAsyncComponent(() =>
  import("./components/CoverScanner.vue").catch((error: unknown) => {
    scan.finish(scan.file);
    throw error;
  }),
);

// The finished moment loads with the first finished book. The status is
// saved before it opens, so a chunk that fails to load only skips it.
const FinishedMoment = defineAsyncComponent(() =>
  import("./components/FinishedMoment.vue").catch((error: unknown) => {
    finish.close(null);
    throw error;
  }),
);
</script>

<template>
  <RouterView />
  <CoverScanner v-if="scan.file" />
  <FinishedMoment v-if="finish.item" :key="finish.serial" />
  <ToastHost />
</template>
