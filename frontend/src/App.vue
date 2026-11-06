<script setup lang="ts">
import CoverScanner from "./components/CoverScanner.vue";
import ToastHost from "./components/ToastHost.vue";
</script>

<template>
  <RouterView />
  <CoverScanner />
  <ToastHost />
</template>
