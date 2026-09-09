<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import Avatar from "../components/Avatar.vue";
import NavIcon from "../components/NavIcon.vue";
import { NAV_ITEMS, isActive } from "../nav";
import { useClub } from "../stores/club";
import { useSession } from "../stores/session";

const { t } = useI18n();
const club = useClub();
const session = useSession();
const route = useRoute();

const bottomItems = NAV_ITEMS.filter((item) => item.bottomBar);
const username = computed(() => session.user?.username ?? "");
const avatarUrl = computed(() => session.user?.avatar_url ?? null);
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <RouterLink class="wordmark" to="/">{{ club.name }}</RouterLink>
      <RouterLink
        class="avatar-link"
        to="/settings"
        :aria-label="t('nav.settingsFor', { name: username })"
      >
        <Avatar :username="username" :src="avatarUrl" />
      </RouterLink>
    </header>

    <aside class="sidebar">
      <RouterLink class="wordmark sidebar-mark" to="/">{{ club.name }}</RouterLink>
      <nav class="sidebar-nav" aria-label="Main">
        <RouterLink
          v-for="item in NAV_ITEMS"
          :key="item.to"
          class="side-link"
          :class="{ active: isActive(route.path, item.to) }"
          :aria-current="isActive(route.path, item.to) ? 'page' : undefined"
          :to="item.to"
        >
          <NavIcon :name="item.icon" :size="20" />
          {{ t(item.labelKey) }}
        </RouterLink>
      </nav>
      <RouterLink class="side-account" to="/settings">
        <Avatar :username="username" :src="avatarUrl" />
        <span class="side-account-text">
          <strong>{{ username }}</strong>
          <span class="finer subtle">{{ t("nav.settingsAndTheme") }}</span>
        </span>
      </RouterLink>
    </aside>

    <main class="main">
      <RouterView />
    </main>

    <nav class="bottom-nav" aria-label="Main">
      <RouterLink
        v-for="item in bottomItems"
        :key="item.to"
        :to="item.to"
        :class="{ active: isActive(route.path, item.to) }"
        :aria-current="isActive(route.path, item.to) ? 'page' : undefined"
      >
        <NavIcon :name="item.icon" />
        {{ t(item.labelKey) }}
      </RouterLink>
    </nav>
  </div>
</template>

<style scoped>
.shell {
  min-height: 100dvh;
}

.topbar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  z-index: 20;
  height: calc(var(--header-h) + env(safe-area-inset-top));
  padding: env(safe-area-inset-top) var(--space-4) 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--bg-translucent);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--border);
}

.avatar-link {
  display: grid;
  place-items: center;
  min-width: var(--tap);
  min-height: var(--tap);
  border-radius: var(--radius-pill);
  text-decoration: none;
}

.main {
  max-width: var(--content-max);
  margin: 0 auto;
  padding: calc(var(--header-h) + env(safe-area-inset-top) + var(--space-4))
    var(--space-4) calc(var(--nav-h) + env(safe-area-inset-bottom) + var(--space-7));
}

.bottom-nav {
  position: fixed;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 20;
  height: calc(var(--nav-h) + env(safe-area-inset-bottom));
  padding: var(--space-1) var(--space-2) env(safe-area-inset-bottom);
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  background: var(--bg-translucent-strong);
  backdrop-filter: blur(12px);
  border-top: 1px solid var(--border);
}

.bottom-nav a {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  text-decoration: none;
  color: var(--text-muted);
  font-size: var(--text-2xs);
  font-weight: 600;
  min-height: var(--tap);
}

.bottom-nav a.active {
  color: var(--accent);
}

.sidebar {
  display: none;
}

@media (min-width: 1024px) {
  .topbar,
  .bottom-nav {
    display: none;
  }

  .shell {
    padding-left: var(--sidebar-w);
  }

  .sidebar {
    position: fixed;
    top: 0;
    bottom: 0;
    left: 0;
    width: var(--sidebar-w);
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding: var(--space-6) var(--space-4) var(--space-4);
    background: var(--surface);
    border-right: 1px solid var(--border);
  }

  .sidebar-mark {
    font-size: var(--text-2xl);
    padding: 0 var(--space-2);
  }

  .sidebar-nav {
    display: grid;
    gap: 2px;
  }

  .side-link {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    color: var(--text-muted);
    font-weight: 600;
    text-decoration: none;
    transition:
      background var(--dur-fast) var(--ease),
      color var(--dur-fast) var(--ease);
  }

  .side-link:hover {
    background: var(--surface-2);
    color: var(--text);
  }

  .side-link.active {
    background: var(--accent-soft);
    color: var(--accent-hover);
  }

  .side-account {
    margin-top: auto;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    color: inherit;
    text-decoration: none;
    min-width: 0;
  }

  .side-account:hover {
    background: var(--surface-2);
  }

  .side-account-text {
    display: grid;
    min-width: 0;
  }

  .side-account-text strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .main {
    padding: var(--space-7) var(--space-7) var(--space-9);
  }
}
</style>
