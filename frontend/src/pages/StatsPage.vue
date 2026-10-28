<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import type { BookOut, PersonalStatsOut } from "../api/generated";
import BackLink from "../components/BackLink.vue";
import BookCover from "../components/BookCover.vue";
import MonthBars from "../components/MonthBars.vue";
import StatTiles from "../components/StatTiles.vue";
import { bookPath } from "../constants";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { formatCount, formatRating } from "../stats";

/**
 * A reader's year in books (`GET /api/stats`): `/stats` for the viewer,
 * `/u/:username/stats` for someone in their audience. `?year=` picks the year.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const locale = useLocale();
const session = useSession();

const username = computed(() => (route.params.username ? String(route.params.username) : ""));
const own = computed(() => !username.value || username.value === session.user?.username);
const stats = ref<PersonalStatsOut | null>(null);
const error = ref("");
const loading = ref(false);

function requestedYear(): number | undefined {
  const year = Number(route.query.year);
  return Number.isInteger(year) && year >= 2000 && year <= 2100 ? year : undefined;
}

async function load() {
  loading.value = true;
  try {
    stats.value = await api.personalStats({ username: own.value ? undefined : username.value, year: requestedYear() });
    error.value = "";
  } catch (err) {
    stats.value = null;
    error.value = err instanceof ApiError ? err.message : t("stats.loadFailed");
  } finally {
    loading.value = false;
  }
}

function chooseYear(event: Event) {
  const year = Number((event.target as HTMLSelectElement).value);
  void router.replace({ query: { ...route.query, year: String(year) } });
}

const tiles = computed(() => {
  const s = stats.value;
  if (!s) return [];
  return [
    { key: "finished", label: t("stats.finished"), value: formatCount(s.finished, locale.locale) },
    { key: "pages", label: t("stats.pages"), value: formatCount(s.pages, locale.locale) },
    { key: "average", label: t("stats.average"), value: formatRating(s.average_rating, locale.locale) },
    { key: "five", label: t("stats.fiveStars"), value: formatCount(s.five_stars, locale.locale) },
    { key: "dnf", label: t("stats.dnf"), value: formatCount(s.dnf, locale.locale) },
    { key: "quotes", label: t("stats.quotes"), value: formatCount(s.quotes, locale.locale) },
  ];
});

const highlights = computed(() => {
  const s = stats.value;
  const rows: { key: string; label: string; book: BookOut }[] = [];
  if (s?.top_book) rows.push({ key: "top", label: t("stats.topBook"), book: s.top_book });
  if (s?.longest_book) rows.push({ key: "longest", label: t("stats.longestBook"), book: s.longest_book });
  return rows;
});

const quiet = computed(() => stats.value && stats.value.finished === 0 && stats.value.dnf === 0);

watch(() => [route.params.username, route.query.year], load, { immediate: true });
</script>

<template>
  <section class="stats-page">
    <BackLink v-if="!own" :label="t('memberShelf.back')" :fallback="`/u/${username}`" />
    <div class="page-head stats-head">
      <div>
        <h1>{{ own ? t("stats.title") : t("stats.titleOf", { name: stats?.username ?? username }) }}</h1>
        <p class="lede">{{ own ? t("stats.lede") : t("stats.ledeOf", { name: stats?.username ?? username }) }}</p>
      </div>
      <label v-if="stats && stats.years.length > 1" class="field year-field">
        <span>{{ t("stats.year") }}</span>
        <select :value="stats.year" :disabled="loading" @change="chooseYear">
          <option v-for="year in stats.years" :key="year" :value="year">{{ year }}</option>
        </select>
      </label>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!stats" class="skeleton stats-skeleton" aria-hidden="true" />
    <template v-else>
      <h2 class="year-heading">{{ stats.year }}</h2>
      <StatTiles :items="tiles" />
      <p v-if="quiet" class="fine subtle quiet">{{ t("stats.nothingYet", { year: stats.year }) }}</p>
      <MonthBars v-else class="section" :year="stats.year" :months="stats.by_month" />

      <section v-if="highlights.length" class="section highlights" aria-labelledby="highlights">
        <h3 id="highlights" class="visually-hidden">{{ t("stats.highlights") }}</h3>
        <RouterLink v-for="row in highlights" :key="row.key" class="highlight" :to="bookPath(row.book)">
          <BookCover :book="row.book" size="sm" />
          <span class="highlight-text">
            <span class="finer subtle">{{ row.label }}</span>
            <strong>{{ row.book.title }}</strong>
            <span v-if="row.book.authors" class="fine subtle">{{ row.book.authors }}</span>
          </span>
        </RouterLink>
      </section>
    </template>
  </section>
</template>

<style scoped>
.stats-head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-3);
}

.year-field {
  min-width: 7rem;
}

.year-heading {
  margin-bottom: var(--space-3);
}

.stats-skeleton {
  height: 260px;
}

.quiet {
  margin-top: var(--space-4);
}

.highlights {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
  gap: var(--space-2);
}

.highlight {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  color: inherit;
  text-decoration: none;
}

.highlight-text {
  display: grid;
  gap: 2px;
  min-width: 0;
}
</style>
