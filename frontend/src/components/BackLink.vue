<script setup lang="ts">
import { useRouter } from "vue-router";

/**
 * "← Back": the previous page in this tab, or `fallback` when the page was
 * opened directly. With `exact`, the label names `fallback` ("← Settings"),
 * so it goes back only when the previous page is that one (its path, without
 * query or hash) and opens `fallback` otherwise: a link that names a page
 * never leads somewhere else.
 */
const props = defineProps<{ label: string; fallback: string; exact?: boolean }>();
const router = useRouter();

function back() {
  const state = typeof window !== "undefined" ? window.history.state : null;
  const previous = state && typeof state.back === "string" ? state.back : null;
  if (previous !== null && (!props.exact || previous.split(/[?#]/)[0] === props.fallback)) router.back();
  else void router.push(props.fallback);
}
</script>

<template>
  <p class="fine back-link">
    <a :href="fallback" @click.prevent="back">{{ label }}</a>
  </p>
</template>
