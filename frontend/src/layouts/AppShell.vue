<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import BottomNav from "../components/BottomNav.vue";
import { useSession } from "../stores/session";

const session = useSession();
const router = useRouter();
const menuOpen = ref(false);

async function logout() {
  menuOpen.value = false;
  await session.logout();
  await router.push("/login");
}

function close(event: MouseEvent) {
  const target = event.target as HTMLElement;
  if (!target.closest(".topbar")) menuOpen.value = false;
}

onMounted(() => document.addEventListener("click", close));
onUnmounted(() => document.removeEventListener("click", close));
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <RouterLink class="wordmark" to="/library">Bookclub</RouterLink>
      <div class="topbar-actions">
        <button class="account-btn" type="button" @click.stop="menuOpen = !menuOpen">
          {{ session.user?.username }}
        </button>
      </div>
      <div v-if="menuOpen" class="menu">
        <RouterLink to="/invites" @click="menuOpen = false">Invites</RouterLink>
        <button type="button" @click="logout">Log out</button>
      </div>
    </header>
    <main class="main">
      <RouterView />
    </main>
    <BottomNav />
  </div>
</template>
