<script setup lang="ts">
import { useRouter } from "vue-router";

/** "← Back": the previous page in this tab, or `fallback` when the page was opened directly. */
const props = defineProps<{ label: string; fallback: string }>();
const router = useRouter();

function back() {
  const state = typeof window !== "undefined" ? window.history.state : null;
  if (state && typeof state.back === "string") router.back();
  else void router.push(props.fallback);
}
</script>

<template>
  <p class="fine back-link">
    <a :href="fallback" @click.prevent="back">{{ label }}</a>
  </p>
</template>
