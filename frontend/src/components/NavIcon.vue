<script setup lang="ts">
import type { NavIconName } from "../nav";

defineProps<{ name: NavIconName; size?: number }>();
</script>

<template>
  <svg
    :width="size ?? 22"
    :height="size ?? 22"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="name === 'home'">
      <path d="M4 11.2 12 4.8l8 6.4" />
      <path d="M6.6 10.2V19.2h10.8v-9" />
    </template>
    <template v-else-if="name === 'discover'">
      <circle cx="11" cy="11" r="6.4" />
      <path d="M15.8 15.8 20.2 20.2" />
    </template>
    <template v-else-if="name === 'shelf'">
      <rect x="3.4" y="4.6" width="4.6" height="14.8" rx="1" />
      <rect x="9.7" y="4.6" width="4.6" height="14.8" rx="1" />
      <path d="M17.4 6.2l3.2.9-2.6 12.5-3.2-.9z" />
    </template>
    <template v-else-if="name === 'club'">
      <circle cx="9" cy="8.6" r="3" />
      <circle cx="16.8" cy="9.8" r="2.3" />
      <path d="M4 18.6c.7-2.7 2.7-4.2 5-4.2s4.3 1.5 5 4.2" />
      <path d="M14.8 18.2c.5-1.9 1.9-3 3.4-3 1 0 2 .5 2.7 1.5" />
    </template>
    <template v-else>
      <circle cx="12" cy="12" r="3" />
      <path
        d="M12 3.5v2M12 18.5v2M4.9 7.8l1.7 1M17.4 15.2l1.7 1M4.9 16.2l1.7-1M17.4 8.8l1.7-1"
      />
    </template>
  </svg>
</template>
