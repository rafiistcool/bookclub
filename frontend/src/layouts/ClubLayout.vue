<script setup lang="ts">
import { ChevronDownIcon } from "@heroicons/vue/24/outline";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import ClubMoreSheet from "../components/ClubMoreSheet.vue";
import ClubSwitchSheet from "../components/ClubSwitchSheet.vue";
import { isClubAdmin, isClubGone, useClubs } from "../stores/clubs";

/**
 * Every `/c/:slug/…` page: a compact bar that sticks under the top bar, with
 * the club's name (it opens the club sheet) and five section tabs, around the
 * section. The quieter sections sit behind "More". The section renders once
 * the club has answered, so it can rely on `useClubs().current`. A 404 (not
 * a member, or no such club) shows "club not found" and drops the slug from
 * the switcher and the last-used club.
 */
const { t } = useI18n();
const route = useRoute();
const clubs = useClubs();

const slug = computed(() => String(route.params.slug ?? ""));
const loading = ref(true);
const error = ref("");
const switching = ref(false);
const showingMore = ref(false);

const club = computed(() => (clubs.current?.slug === slug.value ? clubs.current : null));
const notFound = computed(() => clubs.goneSlug === slug.value);
const name = computed(() => club.value?.name ?? clubs.items.find((row) => row.slug === slug.value)?.name ?? "");
const canManage = computed(() => isClubAdmin(club.value?.role ?? "member"));

const base = computed(() => `/c/${slug.value}`);

const tabs = computed(() => [
  { to: base.value, label: t("clubNav.homeShort"), exact: true },
  { to: `${base.value}/discussions`, label: t("clubNav.discussionsShort") },
  { to: `${base.value}/vote`, label: t("clubNav.voteShort") },
  { to: `${base.value}/members`, label: t("clubNav.membersShort") },
]);

/** The sections behind "More", by path; the tab reads "More" there too. */
const moreSections = computed(() => [
  { to: `${base.value}/overlap`, label: t("clubNav.overlap") },
  { to: `${base.value}/events`, label: t("clubNav.events") },
  { to: `${base.value}/stats`, label: t("clubNav.stats") },
]);

function isCurrent(tab: { to: string; exact?: boolean }): boolean {
  if (tab.exact) return route.path === tab.to || route.path === `${tab.to}/`;
  return route.path === tab.to || route.path.startsWith(`${tab.to}/`);
}

const moreSection = computed(() => moreSections.value.find((row) => isCurrent(row)) ?? null);

/** Where the underline sits: 0–3 for the tabs, 4 for "More". */
const activeTab = computed(() => {
  if (moreSection.value) return 4;
  return tabs.value.findIndex((tab) => isCurrent(tab));
});
// The underline stays where it last was while no tab matches (an unknown path).
const inkTab = ref(Math.max(0, activeTab.value));
watch(activeTab, (index) => {
  if (index >= 0) inkTab.value = index;
});

// A new club, section or section anchor closes the sheets (a row in them
// already did); so does Back between two anchors of one page. A query alone
// (the year on the stats page) leaves them.
watch(
  [() => route.path, () => route.hash],
  () => {
    switching.value = false;
    showingMore.value = false;
  },
);

async function load() {
  const current = slug.value;
  if (!current) return;
  loading.value = true;
  error.value = "";
  if (clubs.goneSlug !== current) clubs.goneSlug = null;
  if (clubs.current?.slug !== current) clubs.current = null;
  try {
    const answer = await api.club(current);
    if (slug.value !== current) return;
    clubs.current = answer;
    clubs.upsert(answer);
    clubs.remember(current);
  } catch (err) {
    if (slug.value !== current) return;
    if (isClubGone(err)) clubs.markGone(current);
    else error.value = err instanceof ApiError ? err.message : t("club.loadFailed");
  } finally {
    if (slug.value === current) loading.value = false;
  }
}

watch(slug, () => void load(), { immediate: true });
</script>

<template>
  <section class="club-page">
    <div v-if="notFound" class="empty club-missing">
      <h1>{{ t("club.notFoundTitle") }}</h1>
      <p>{{ t("club.notFoundBody") }}</p>
      <div class="btn-row">
        <RouterLink class="btn btn-primary" to="/clubs">{{ t("club.toYourClubs") }}</RouterLink>
        <RouterLink class="btn btn-ghost" to="/">{{ t("club.toHome") }}</RouterLink>
      </div>
    </div>

    <template v-else>
      <!-- The sheets stay outside the bar: its backdrop blur would trap their fixed position. -->
      <header class="club-head">
        <h1 class="club-title">
          <button
            class="club-name-btn"
            type="button"
            aria-haspopup="dialog"
            :aria-expanded="switching"
            @click="switching = true"
          >
            <span class="club-name">{{ name }}</span>
            <ChevronDownIcon class="club-chevron" aria-hidden="true" />
          </button>
        </h1>

        <nav class="club-tabs" :aria-label="t('clubNav.label')" :style="{ '--tab': inkTab }">
          <RouterLink
            v-for="tab in tabs"
            :key="tab.to"
            :to="tab.to"
            class="club-tab"
            :class="{ active: isCurrent(tab) }"
            :aria-current="isCurrent(tab) ? 'page' : undefined"
          >
            <span class="club-tab-label">{{ tab.label }}</span>
          </RouterLink>
          <button
            class="club-tab more-tab"
            :class="{ active: moreSection }"
            type="button"
            aria-haspopup="dialog"
            :aria-expanded="showingMore"
            :aria-label="moreSection ? t('clubNav.moreOn', { section: moreSection.label }) : undefined"
            @click="showingMore = true"
          >
            <span class="club-tab-label">{{ t("clubNav.more") }}</span>
          </button>
          <span class="tab-ink" :class="{ off: activeTab < 0 }" aria-hidden="true" />
        </nav>
      </header>

      <ClubSwitchSheet v-if="switching" :slug="slug" @close="switching = false" />
      <ClubMoreSheet
        v-if="showingMore"
        :slug="slug"
        :can-manage="canManage"
        @close="showingMore = false"
      />

      <p v-if="error" class="error">
        {{ error }}
        <button class="text-btn" type="button" @click="load">{{ t("common.tryAgain") }}</button>
      </p>
      <div v-else-if="loading && !club" class="skeleton club-skeleton" aria-hidden="true" />
      <RouterView v-else-if="club" :key="slug" />
    </template>
  </section>
</template>

<style scoped>
/* One bar under the app's top bar, full bleed on the phone: the name, then
   the tabs, closed by one hairline. Nearly opaque (like the sticky composer),
   so text scrolling under the tab labels doesn't show through. */
.club-head {
  position: sticky;
  top: calc(var(--header-h) + env(safe-area-inset-top));
  z-index: 15;
  margin: calc(-1 * var(--space-4)) calc(-1 * var(--space-4)) var(--space-5);
  padding: 0 var(--space-4);
  background: var(--bg-translucent-strong);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--border);
}

.club-title {
  margin: 0;
  font: inherit;
}

.club-name-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  max-width: 100%;
  min-height: 48px;
  margin-left: calc(-1 * var(--space-1));
  padding: var(--space-1);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font: inherit;
  cursor: pointer;
  transition: transform var(--dur-fast) var(--ease-out);
}

.club-name-btn:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.club-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--serif);
  font-size: var(--text-xl);
  font-weight: 600;
  line-height: var(--leading-tight);
  letter-spacing: var(--tracking-tight);
}

.club-chevron {
  flex: none;
  width: 18px;
  height: 18px;
  margin-top: 2px;
  color: var(--text-muted);
  stroke-width: 2;
}

@media (hover: hover) {
  .club-name-btn:hover .club-chevron {
    color: var(--accent);
  }
}

/* Five equal tabs that fit 320 px without scrolling. */
.club-tabs {
  position: relative;
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  margin: 0 calc(-1 * var(--space-2));
}

.club-tab {
  display: grid;
  place-items: center;
  min-width: 0;
  min-height: var(--tap);
  padding: 0 2px;
  border: 0;
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-weight: 600;
  font-size: var(--text-sm);
  text-decoration: none;
  cursor: pointer;
  transition: color var(--dur-base) var(--ease-out);
}

.club-tab-label {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: transform var(--dur-fast) var(--ease-out);
}

.club-tab:active .club-tab-label {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .club-tab:hover {
    color: var(--text);
  }
}

.club-tab.active {
  color: var(--accent);
}

/* The underline slides from tab to tab. */
.tab-ink {
  position: absolute;
  left: 0;
  bottom: -1px;
  width: 20%;
  height: 2px;
  transform: translateX(calc(var(--tab) * 100%));
  transition:
    transform var(--dur-slow) var(--ease-out),
    opacity var(--dur-base) var(--ease);
  pointer-events: none;
}

.tab-ink::before {
  content: "";
  position: absolute;
  inset: 0 22%;
  border-radius: var(--radius-pill);
  background: var(--accent);
}

.tab-ink.off {
  opacity: 0;
}

/* Sections on the members page land under the bar, not behind it. */
.club-page :deep([id]) {
  scroll-margin-top: calc(var(--header-h) + env(safe-area-inset-top) + 108px);
}

.club-skeleton {
  height: 220px;
}

.club-missing h1 {
  font-size: var(--text-xl);
}

@media (max-width: 339px) {
  .club-tab {
    font-size: var(--text-xs);
  }
}

@media (min-width: 720px) {
  .club-name {
    font-size: var(--text-2xl);
  }
}

/* No top bar on the desktop: the bar sticks to the top, inside the column. */
@media (min-width: 1024px) {
  .club-head {
    top: 0;
    margin: calc(-1 * var(--space-7)) 0 var(--space-6);
    padding: var(--space-5) 0 0;
  }

  .club-tabs {
    max-width: 560px;
    margin: 0;
  }

  .club-page :deep([id]) {
    scroll-margin-top: 140px;
  }
}
</style>
