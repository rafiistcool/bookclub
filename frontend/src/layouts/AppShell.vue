<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import Avatar from "../components/Avatar.vue";
import BookFlow from "../components/BookFlow.vue";
import BottomNav from "../components/BottomNav.vue";
import NavIcon from "../components/NavIcon.vue";
import { useClub } from "../stores/club";
import { usePick } from "../stores/pick";
import { usePush } from "../stores/push";
import { useSession } from "../stores/session";

const club = useClub();
const session = useSession();
const route = useRoute();
const router = useRouter();

const title = computed(() => {
  if (route.name === "member" && typeof route.params.username === "string") {
    return route.params.username;
  }
  return (route.meta.title as string | undefined) ?? club.name;
});
const back = computed(() => route.meta.back as string | undefined);
const wide = computed(() => Boolean(route.meta.wide));

async function goBack() {
  if (window.history.length > 1) router.back();
  else await router.push(back.value ?? "/");
}

const links = [
  { to: "/", label: "Home", icon: "home", exact: true },
  { to: "/library", label: "Library", icon: "search" },
  { to: "/shelf", label: "Shelf", icon: "shelf" },
  { to: "/friends", label: "Club", icon: "people" },
] as const;

function active(prefix: string, exact = false) {
  if (exact) return route.path === "/";
  return route.path === prefix || route.path.startsWith(prefix + "/");
}

onMounted(() => {
  void usePick().load();
  void usePush().init();
});
</script>

<template>
  <div class="shell">
    <nav class="rail" aria-label="Main">
      <RouterLink class="wordmark" to="/">{{ club.name }}</RouterLink>
      <RouterLink
        v-for="link in links"
        :key="link.to"
        :to="link.to"
        :class="{ active: active(link.to, 'exact' in link && link.exact) }"
      >
        <NavIcon :name="link.icon" />
        {{ link.label }}
      </RouterLink>
      <div class="rail-foot">
        <RouterLink to="/settings" :class="{ active: active('/settings') }">
          <Avatar :username="session.user?.username ?? '?'" size="sm" />
          {{ session.user?.username }}
        </RouterLink>
      </div>
    </nav>

    <header class="topbar">
      <button v-if="back" class="icon-btn topbar-back" type="button" aria-label="Back" @click="goBack">
        <NavIcon name="back" />
      </button>
      <h1 class="topbar-title">{{ title }}</h1>
      <RouterLink
        class="icon-btn"
        to="/settings"
        aria-label="Settings"
        :aria-current="active('/settings') ? 'page' : undefined"
      >
        <Avatar :username="session.user?.username ?? '?'" size="sm" />
      </RouterLink>
    </header>

    <main class="main" :class="{ wide }">
      <RouterView />
    </main>

    <BottomNav />
    <BookFlow />
  </div>
</template>
