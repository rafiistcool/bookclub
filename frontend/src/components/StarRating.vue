<script setup lang="ts">
/** Read-only stars (SVG, accent colour). For input use StarInput. */
defineProps<{
  value: number | null | undefined;
  size?: "sm" | "lg";
}>();
</script>

<template>
  <span v-if="value" class="stars" :class="size" :aria-label="`${value} of 5 stars`" role="img">
    <svg v-for="n in 5" :key="n" viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M12 3.6l2.6 5.5 6 .7-4.4 4.1 1.2 5.9L12 16.9l-5.4 2.9 1.2-5.9L3.4 9.8l6-.7z"
        :fill="n <= value ? 'currentColor' : 'none'"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linejoin="round"
      />
    </svg>
  </span>
</template>
