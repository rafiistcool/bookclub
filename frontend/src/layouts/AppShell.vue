<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import Avatar from "../components/Avatar.vue";
import EmailPrompt from "../components/EmailPrompt.vue";
import LegalLinks from "../components/LegalLinks.vue";
import NavIcon from "../components/NavIcon.vue";
import TermsPrompt from "../components/TermsPrompt.vue";
import { tp } from "../i18n";
import { NAV_ITEMS, isActive } from "../nav";
import { reloadWhenVisible, useFriends } from "../stores/friends";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";

const { t } = useI18n();
const service = useService();
const session = useSession();
const friends = useFriends();
const route = useRoute();

const bottomItems = NAV_ITEMS.filter((item) => item.bottomBar);
const activeTab = computed(() => bottomItems.findIndex((item) => isActive(route.path, item)));
// The pill stays where it last was while a page outside the tabs is open.
const pillTab = ref(Math.max(0, activeTab.value));
watch(activeTab, (index) => {
  if (index >= 0) pillTab.value = index;
});

const view = ref<HTMLElement | null>(null);

function matches(query: string): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia(query).matches;
}

// A new page arrives with a short move that says where it came from: across
// for a sibling tab (from the side the tab sits on), up for a page opened
// from this one. Only the path counts: a query change (a tab inside a page)
// is the page's own business.
watch(
  () => route.path,
  (now, before) => {
    const element = view.value;
    if (!element || typeof element.animate !== "function" || now === before) return;
    const from = bottomItems.findIndex((item) => isActive(before, item));
    const to = bottomItems.findIndex((item) => isActive(now, item));
    if (matches("(prefers-reduced-motion: reduce)")) {
      element.animate([{ opacity: 0.4 }, { opacity: 1 }], { duration: 160, easing: "ease-out" });
      return;
    }
    const across = from >= 0 && to >= 0 && from !== to;
    const start = across ? `translateX(${to > from ? 24 : -24}px)` : "translateY(12px)";
    element.animate(
      [
        { opacity: 0, transform: start },
        { opacity: 1, transform: "none" },
      ],
      { duration: across ? 320 : 280, easing: "cubic-bezier(0.16, 1, 0.3, 1)" },
    );
  },
  { flush: "post" },
);
const username = computed(() => session.user?.username ?? "");
const avatarUrl = computed(() => session.user?.avatar_url ?? null);

/** The Friends item's count: requests and club invitations waiting (Home shows it on the phone). */
function badgeFor(to: string): number {
  return to === "/friends" ? friends.badge : 0;
}

let stopReloading: (() => void) | null = null;

onMounted(() => {
  void friends.load();
  stopReloading = reloadWhenVisible();
});

onUnmounted(() => stopReloading?.());

// A join, leave or new club changes which invitations still wait. A sign-out
// (no user) asks nothing.
watch(
  () => session.user?.clubs.map((club) => club.slug).sort().join(" "),
  (now, before) => {
    if (session.user && now !== before) void friends.load();
  },
);
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <RouterLink class="wordmark" to="/">{{ service.name }}</RouterLink>
      <RouterLink
        class="avatar-link"
        to="/settings"
        :aria-label="t('nav.settingsFor', { name: username })"
      >
        <Avatar :username="username" :src="avatarUrl" />
      </RouterLink>
    </header>

    <aside class="sidebar">
      <RouterLink class="wordmark sidebar-mark" to="/">{{ service.name }}</RouterLink>
      <nav class="sidebar-nav" :aria-label="t('nav.main')">
        <RouterLink
          v-for="item in NAV_ITEMS"
          :key="item.to"
          class="side-link"
          :class="{ active: isActive(route.path, item) }"
          :aria-current="isActive(route.path, item) ? 'page' : undefined"
          :to="item.to"
        >
          <NavIcon :name="item.icon" :size="20" />
          {{ t(item.labelKey) }}
          <span v-if="badgeFor(item.to)" class="side-count nums">
            <span aria-hidden="true">{{ badgeFor(item.to) }}</span>
            <span class="visually-hidden">{{ tp("friends.waiting", badgeFor(item.to)) }}</span>
          </span>
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
      <TermsPrompt />
      <EmailPrompt />
      <div ref="view" class="view">
        <RouterView />
      </div>
      <!-- Settings lists the same texts in its Legal section. -->
      <footer v-if="service.legal.length && route.path !== '/settings'" class="shell-footer">
        <LegalLinks />
      </footer>
    </main>

    <nav class="bottom-nav" :aria-label="t('nav.main')">
      <span
        class="nav-pill"
        :class="{ off: activeTab < 0 }"
        :style="{ '--tab': pillTab }"
        aria-hidden="true"
      />
      <RouterLink
        v-for="item in bottomItems"
        :key="item.to"
        :to="item.to"
        :class="{ active: isActive(route.path, item) }"
        :aria-current="isActive(route.path, item) ? 'page' : undefined"
      >
        <NavIcon :name="item.icon" />
        {{ t(item.shortLabelKey ?? item.labelKey) }}
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

.shell-footer {
  margin-top: var(--space-8);
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
  grid-template-columns: repeat(5, 1fr);
  background: var(--bg-translucent-strong);
  backdrop-filter: blur(12px);
  border-top: 1px solid var(--border);
}

.bottom-nav a {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  gap: 4px;
  padding-top: 8px;
  text-decoration: none;
  color: var(--text-muted);
  font-size: var(--text-2xs);
  font-weight: 600;
  min-height: var(--tap);
  transition: color var(--dur-base) var(--ease-out);
}

.bottom-nav a :deep(svg) {
  transition: transform var(--dur-fast) var(--ease-out);
}

.bottom-nav a:active :deep(svg) {
  transform: scale(0.86);
  transition-duration: var(--dur-press);
}

.bottom-nav a.active {
  color: var(--accent-hover);
}

/* The active tab's pill slides from tab to tab under the icons. */
.nav-pill {
  position: absolute;
  top: calc(var(--space-1) + 4px);
  left: var(--space-2);
  width: calc((100% - 2 * var(--space-2)) / 5);
  height: 32px;
  transform: translateX(calc(var(--tab) * 100%));
  transition:
    transform var(--dur-slow) var(--ease-out),
    opacity var(--dur-base) var(--ease);
  pointer-events: none;
}

.nav-pill::before {
  content: "";
  position: absolute;
  inset: 0;
  width: 58px;
  margin: 0 auto;
  border-radius: var(--radius-pill);
  background: var(--accent-soft);
}

.nav-pill.off {
  opacity: 0;
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

  @media (hover: hover) {
    .side-link:hover {
      background: var(--surface-2);
      color: var(--text);
    }
  }

  .side-link.active {
    background: var(--accent-soft);
    color: var(--accent-hover);
  }

  .side-count {
    margin-left: auto;
    min-width: 1.5rem;
    padding: 0 var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--accent);
    color: var(--accent-ink);
    font-size: var(--text-xs);
    font-weight: 700;
    line-height: 1.5rem;
    text-align: center;
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

  @media (hover: hover) {
    .side-account:hover {
      background: var(--surface-2);
    }
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
