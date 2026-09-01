<script setup lang="ts">
import { useRoute } from "vue-router";
import NavIcon from "./NavIcon.vue";

const route = useRoute();

function active(prefix: string) {
  if (prefix === "/") return route.path === "/";
  return route.path === prefix || route.path.startsWith(prefix + "/");
}
</script>

<template>
  <nav class="bottom-nav" aria-label="Main">
    <RouterLink to="/" :class="{ active: active('/') }" :aria-current="active('/') ? 'page' : undefined">
      <NavIcon name="home" />
      Home
    </RouterLink>
    <RouterLink to="/library" :class="{ active: active('/library') }" :aria-current="active('/library') ? 'page' : undefined">
      <NavIcon name="search" />
      Library
    </RouterLink>
    <RouterLink to="/shelf" :class="{ active: active('/shelf') }" :aria-current="active('/shelf') ? 'page' : undefined">
      <NavIcon name="shelf" />
      Shelf
    </RouterLink>
    <RouterLink to="/friends" :class="{ active: active('/friends') }" :aria-current="active('/friends') ? 'page' : undefined">
      <NavIcon name="people" />
      Club
    </RouterLink>
  </nav>
</template>
