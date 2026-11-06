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
    <template v-else-if="name === 'friends'">
      <circle cx="8.6" cy="8.4" r="3" />
      <path d="M3.6 18.6c.7-2.7 2.7-4.2 5-4.2 1 0 1.9.3 2.7.8" />
      <path d="M17.5 19.6c-2.6-1.6-4.2-3.3-4.2-5 0-1.2.9-2.1 2-2.1.9 0 1.6.5 2.2 1.3.6-.8 1.3-1.3 2.2-1.3 1.1 0 2 .9 2 2.1 0 1.7-1.6 3.4-4.2 5z" />
    </template>
    <template v-else-if="name === 'profile'">
      <circle cx="12" cy="8.6" r="3.4" />
      <path d="M5.4 19.4c.9-3.2 3.5-5 6.6-5s5.7 1.8 6.6 5" />
    </template>
    <!-- Settings: a cog (the earlier rays around a dot read as a sun). -->
    <template v-else>
      <path
        d="M9.6 3.9c.1-.5.6-.9 1.1-.9h2.6c.5 0 1 .4 1.1.9l.2 1.3c.1.4.3.7.6.9l.2.1c.3.2.7.3 1.1.1l1.2-.5c.5-.2 1.1 0 1.4.5l1.3 2.2c.3.5.2 1.1-.3 1.4l-1 .8c-.3.2-.4.6-.4 1v.3c0 .4.1.8.4 1l1 .8c.4.4.5 1 .3 1.4l-1.3 2.2c-.3.5-.9.7-1.4.5l-1.2-.5c-.4-.1-.8-.1-1.1.1l-.2.1c-.3.2-.6.5-.6.9l-.2 1.3c-.1.5-.6.9-1.1.9h-2.6c-.5 0-1-.4-1.1-.9l-.2-1.3c-.1-.4-.3-.7-.6-.9l-.2-.1c-.3-.2-.7-.3-1.1-.1l-1.2.5c-.5.2-1.1 0-1.4-.5l-1.3-2.2c-.3-.5-.2-1.1.3-1.4l1-.8c.3-.2.4-.6.4-1v-.3c0-.4-.1-.8-.4-1l-1-.8c-.4-.4-.5-1-.3-1.4l1.3-2.2c.3-.5.9-.7 1.4-.5l1.2.5c.4.1.8.1 1.1-.1l.2-.1c.3-.2.6-.5.6-.9z"
      />
      <circle cx="12" cy="12" r="3" />
    </template>
  </svg>
</template>
