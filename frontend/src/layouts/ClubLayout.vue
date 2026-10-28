<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import ClubSwitcher from "../components/ClubSwitcher.vue";
import { isClubAdmin, isClubGone, useClubs } from "../stores/clubs";
import { tp } from "../i18n";

/**
 * Every `/c/:slug/…` page: the club's name, the switcher and the section
 * tabs, around the section. The section renders once the club has answered,
 * so it can rely on `useClubs().current`. A 404 (not a member, or no such
 * club) shows "club not found" and drops the slug from the switcher and the
 * last-used club.
 */
const { t } = useI18n();
const route = useRoute();
const clubs = useClubs();

const slug = computed(() => String(route.params.slug ?? ""));
const loading = ref(true);
const error = ref("");

const club = computed(() => (clubs.current?.slug === slug.value ? clubs.current : null));
const notFound = computed(() => clubs.goneSlug === slug.value);

const tabs = computed(() => {
  const base = `/c/${slug.value}`;
  return [
    { to: base, label: t("clubNav.home"), exact: true },
    { to: `${base}/discussions`, label: t("clubNav.discussions") },
    { to: `${base}/vote`, label: t("clubNav.vote") },
    { to: `${base}/members`, label: t("clubNav.members") },
    { to: `${base}/overlap`, label: t("clubNav.overlap") },
    { to: `${base}/events`, label: t("clubNav.events") },
  ];
});

function isCurrent(tab: { to: string; exact?: boolean }): boolean {
  if (tab.exact) return route.path === tab.to || route.path === `${tab.to}/`;
  return route.path === tab.to || route.path.startsWith(`${tab.to}/`);
}

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
      <header class="club-head">
        <div class="club-title">
          <p class="kicker">{{ t("club.kicker") }}</p>
          <h1 class="display club-name">{{ club?.name ?? clubs.items.find((row) => row.slug === slug)?.name ?? "" }}</h1>
          <p v-if="club" class="fine subtle">
            {{ tp("club.memberCount", club.member_count) }}
            <template v-if="isClubAdmin(club.role)"> · {{ t(`roles.${club.role}`) }}</template>
          </p>
        </div>
        <ClubSwitcher :slug="slug" />
      </header>

      <nav class="club-tabs" :aria-label="t('clubNav.label')">
        <RouterLink
          v-for="tab in tabs"
          :key="tab.to"
          :to="tab.to"
          class="club-tab"
          :class="{ active: isCurrent(tab) }"
          :aria-current="isCurrent(tab) ? 'page' : undefined"
        >
          {{ tab.label }}
        </RouterLink>
      </nav>

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
.club-head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-3);
}

.club-title {
  display: grid;
  gap: var(--space-1);
  min-width: 0;
}

.club-name {
  font-size: var(--text-2xl);
  overflow-wrap: anywhere;
}

.club-tabs {
  display: flex;
  gap: var(--space-1);
  overflow-x: auto;
  scrollbar-width: none;
  margin: 0 calc(-1 * var(--space-4)) var(--space-5);
  padding: 0 var(--space-4);
  border-bottom: 1px solid var(--border);
}

.club-tabs::-webkit-scrollbar {
  display: none;
}

.club-tab {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  padding: 0 var(--space-3);
  color: var(--text-muted);
  font-weight: 600;
  font-size: var(--text-sm);
  text-decoration: none;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
}

.club-tab:hover {
  color: var(--text);
}

.club-tab.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

.club-skeleton {
  height: 220px;
}

.club-missing h1 {
  font-size: var(--text-xl);
}

@media (min-width: 720px) {
  .club-name {
    font-size: var(--text-3xl);
  }
}

@media (min-width: 1024px) {
  .club-tabs {
    margin-left: 0;
    margin-right: 0;
    padding: 0;
  }
}
</style>
