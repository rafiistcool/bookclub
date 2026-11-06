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
import { NAV_ITEMS, PROFILE_ITEM, isActive } from "../nav";
import { reloadWhenVisible, useFriends } from "../stores/friends";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";

const { t } = useI18n();
const service = useService();
const session = useSession();
const friends = useFriends();
const route = useRoute();

const bottomItems = NAV_ITEMS.filter((item) => item.bottomBar);
const sideItems = NAV_ITEMS.filter((item) => item.sidebar !== false);
const onProfile = computed(() => isActive(route.path, PROFILE_ITEM));
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
// is the page's own business, and so is a section change inside one club:
// the club's sticky bar stays put and its tab underline moves instead.
function clubOf(path: string): string | null {
  return /^\/c\/([^/]+)/.exec(path)?.[1] ?? null;
}

watch(
  () => route.path,
  (now, before) => {
    const element = view.value;
    if (!element || typeof element.animate !== "function" || now === before) return;
    const club = clubOf(now);
    if (club && club === clubOf(before)) return;
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
    </header>

    <aside class="sidebar">
      <RouterLink class="wordmark sidebar-mark" to="/">{{ service.name }}</RouterLink>
      <nav class="sidebar-nav" :aria-label="t('nav.main')">
        <RouterLink
          v-for="item in sideItems"
          :key="item.to"
          class="side-link"
          :class="{ active: isActive(route.path, item) }"
          :aria-current="isActive(route.path, item) ? 'page' : undefined"
          :to="item.to"
        >
          <NavIcon :name="item.icon" :size="20" />
          {{ t(item.labelKey) }}
        </RouterLink>
      </nav>
      <!-- The profile, with Friends and Settings under it; the gear goes straight to Settings. -->
      <div class="side-account" :class="{ active: onProfile }">
        <RouterLink
          class="side-account-link"
          :to="PROFILE_ITEM.to"
          :aria-current="onProfile ? 'page' : undefined"
        >
          <Avatar :username="username" :src="avatarUrl" />
          <span class="side-account-text">
            <strong>{{ username }}</strong>
            <span class="finer subtle">{{ t(PROFILE_ITEM.labelKey) }}</span>
          </span>
          <span v-if="friends.badge" class="side-count nums">
            <span aria-hidden="true">{{ friends.badge }}</span>
            <span class="visually-hidden">{{ tp("friends.waiting", friends.badge) }}</span>
          </span>
        </RouterLink>
        <RouterLink
          class="icon-btn side-settings"
          :class="{ on: isActive(route.path, '/settings') }"
          to="/settings"
          :aria-label="t('nav.settings')"
        >
          <NavIcon name="settings" :size="20" />
        </RouterLink>
      </div>
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
        <!-- The profile tab is the reader's own picture: a ring when open, a dot while friends wait. -->
        <span v-if="item === PROFILE_ITEM" class="tab-avatar" :class="{ waiting: friends.badge > 0 }">
          <Avatar :username="username" :src="avatarUrl" size="sm" />
        </span>
        <NavIcon v-else :name="item.icon" />
        {{ t(item.shortLabelKey ?? item.labelKey) }}
        <span v-if="item === PROFILE_ITEM && friends.badge" class="visually-hidden">
          {{ tp("friends.waiting", friends.badge) }}
        </span>
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

.main {
  max-width: var(--content-max);
  margin: 0 auto;
  padding: calc(var(--header-h) + env(safe-area-inset-top) + var(--space-4))
    var(--space-4) calc(var(--nav-h) + env(safe-area-inset-bottom) + var(--space-7));
}

/* The club bar pulls itself up to the column's top edge. Under a Terms or
   Email prompt it starts below the prompt instead of over its buttons. */
.main > .panel ~ .view :deep(.club-head) {
  margin-top: 0;
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

.bottom-nav a :deep(svg),
.tab-avatar {
  transition: transform var(--dur-fast) var(--ease-out);
}

.bottom-nav a:active :deep(svg),
.bottom-nav a:active .tab-avatar {
  transform: scale(0.86);
  transition-duration: var(--dur-press);
}

/* 24px, in the 22px icons' slot; the ring sits outside it on the page colour. */
.tab-avatar {
  position: relative;
  display: grid;
  margin-top: -1px;
  border-radius: var(--radius-pill);
  box-shadow: 0 0 0 1.5px transparent;
}

.bottom-nav a.active .tab-avatar,
.side-account.active .side-account-link :deep(.avatar) {
  box-shadow:
    0 0 0 2px var(--bg),
    0 0 0 3.5px var(--accent-hover);
}

.tab-avatar.waiting::after {
  content: "";
  position: absolute;
  top: -2px;
  right: -3px;
  width: 10px;
  height: 10px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  box-shadow: 0 0 0 2px var(--bg);
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
    flex: none;
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
    gap: var(--space-1);
    padding: var(--space-1);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    min-width: 0;
    transition: background var(--dur-fast) var(--ease);
  }

  .side-account.active {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }

  .side-account-link {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    color: inherit;
    text-decoration: none;
    min-width: 0;
  }

  @media (hover: hover) {
    .side-account:not(.active) .side-account-link:hover {
      background: var(--surface-2);
    }
  }

  .side-settings {
    flex: none;
    text-decoration: none;
  }

  .side-settings.on {
    color: var(--accent-hover);
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
