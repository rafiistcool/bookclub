<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    username: string;
    src?: string | null;
    size?: "xs" | "sm" | "md" | "lg";
  }>(),
  { src: null, size: "md" },
);

const initial = computed(() => props.username.charAt(0).toUpperCase() || "?");
</script>

<template>
  <span class="avatar" :class="`size-${size}`" aria-hidden="true">
    <img v-if="src" :src="src" alt="" />
    <template v-else>{{ initial }}</template>
  </span>
</template>

<style scoped>
.avatar {
  display: grid;
  place-items: center;
  border-radius: var(--radius-pill);
  background: var(--accent);
  color: var(--accent-ink);
  font-family: var(--serif);
  font-weight: 700;
  overflow: hidden;
  flex: 0 0 auto;
}

.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.size-xs {
  width: 20px;
  height: 20px;
  font-size: var(--text-2xs);
}

.size-sm {
  width: 24px;
  height: 24px;
  font-size: var(--text-xs);
}

.size-md {
  width: 34px;
  height: 34px;
  font-size: var(--text-sm);
}

.size-lg {
  width: 72px;
  height: 72px;
  font-size: var(--text-2xl);
}
</style>
