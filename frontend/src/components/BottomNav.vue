<script setup lang="ts">
import { useRoute } from "vue-router";

const route = useRoute();

function active(prefix: string) {
  return route.path === prefix || route.path.startsWith(prefix + "/");
}
</script>

<template>
  <nav class="bottom-nav" aria-label="Main">
    <RouterLink to="/library" :class="{ active: active('/library') }">
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <circle cx="11" cy="11" r="6.5" stroke="currentColor" stroke-width="1.8" />
        <path d="M16 16.5 20 20.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
      </svg>
      Library
    </RouterLink>
    <RouterLink to="/shelf" :class="{ active: active('/shelf') }">
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <rect x="3.5" y="4.5" width="5" height="15" rx="1" stroke="currentColor" stroke-width="1.8" />
        <rect x="9.5" y="4.5" width="5" height="15" rx="1" stroke="currentColor" stroke-width="1.8" />
        <rect x="15.5" y="4.5" width="5" height="15" rx="1" stroke="currentColor" stroke-width="1.8" />
      </svg>
      Shelf
    </RouterLink>
    <RouterLink to="/friends" :class="{ active: active('/friends') }">
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <circle cx="9" cy="9" r="3" stroke="currentColor" stroke-width="1.8" />
        <circle cx="16.5" cy="10" r="2.4" stroke="currentColor" stroke-width="1.8" />
        <path
          d="M4.5 18c.6-2.6 2.6-4 4.5-4s3.9 1.4 4.5 4"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
        />
        <path
          d="M14 17.6c.5-1.8 1.9-2.8 3.3-2.8 1.3 0 2.5.8 3.2 2.4"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
        />
      </svg>
      Friends
    </RouterLink>
  </nav>
</template>
