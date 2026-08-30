<script setup lang="ts">
defineProps<{ count?: number }>();
</script>

<template>
  <span
    v-for="index in count ?? 6"
    :key="index"
    class="book-tile"
    aria-hidden="true"
  >
    <span class="skeleton skeleton-cover" />
    <span class="skeleton skeleton-line" />
    <span class="skeleton skeleton-line short" />
  </span>
</template>
