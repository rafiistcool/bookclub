<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../../api/client";
import type { ClubStatsOut, PickStat } from "../../api/generated";
import BookCover from "../../components/BookCover.vue";
import MonthBars from "../../components/MonthBars.vue";
import StatTiles from "../../components/StatTiles.vue";
import { bookPath } from "../../constants";
import { formatDay } from "../../i18n/dates";
import { isClubGone, useClubs, useCurrentClub } from "../../stores/clubs";
import { useLocale } from "../../stores/locale";
import { formatCount, formatRating } from "../../stats";

/** The club's year in books (`GET /api/clubs/{slug}/stats`): its members' reading and its picks. */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const locale = useLocale();
const clubs = useClubs();
const club = useCurrentClub();
const slug = computed(() => club.value.slug);

const stats = ref<ClubStatsOut | null>(null);
const error = ref("");
const loading = ref(false);

function requestedYear(): number | undefined {
  const year = Number(route.query.year);
  return Number.isInteger(year) && year >= 2000 && year <= 2100 ? year : undefined;
}

async function load() {
  loading.value = true;
  try {
    stats.value = await api.clubStats(slug.value, requestedYear());
    error.value = "";
  } catch (err) {
    if (isClubGone(err)) clubs.markGone(slug.value);
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
    { key: "finished", label: t("stats.clubFinished"), value: formatCount(s.club_finished, locale.locale) },
    { key: "pages", label: t("stats.pages"), value: formatCount(s.club_pages, locale.locale) },
    { key: "average", label: t("stats.average"), value: formatRating(s.club_average_rating, locale.locale) },
    { key: "picks", label: t("stats.picks"), value: formatCount(s.picks.length, locale.locale) },
  ];
});

function pickLine(pick: PickStat): string {
  return t("stats.pickLine", {
    readers: pick.readers,
    finished: pick.finished,
    rating: formatRating(pick.average_rating, locale.locale),
  });
}

function pickWhen(pick: PickStat): string {
  const from = formatDay(pick.started_at, club.value.timezone, locale.locale);
  return pick.ended_at ? `${from} – ${formatDay(pick.ended_at, club.value.timezone, locale.locale)}` : t("stats.pickSince", { day: from });
}

watch(() => [slug.value, route.query.year], load, { immediate: true });
</script>

<template>
  <section class="club-stats" aria-labelledby="club-stats">
    <div class="section-head stats-head">
      <h2 id="club-stats">{{ stats ? t("stats.clubTitle", { year: stats.year }) : t("stats.title") }}</h2>
      <label v-if="stats && stats.years.length > 1" class="year-field">
        <span class="visually-hidden">{{ t("stats.year") }}</span>
        <select :value="stats.year" :disabled="loading" @change="chooseYear">
          <option v-for="year in stats.years" :key="year" :value="year">{{ year }}</option>
        </select>
      </label>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!stats" class="skeleton stats-skeleton" aria-hidden="true" />
    <template v-else>
      <StatTiles :items="tiles" />
      <MonthBars class="section" :year="stats.year" :months="stats.by_month" />

      <section class="section" aria-labelledby="stats-members">
        <div class="section-head">
          <h3 id="stats-members">{{ t("stats.members") }}</h3>
        </div>
        <div class="table-wrap">
          <table class="member-table">
            <thead>
              <tr>
                <th scope="col">{{ t("stats.reader") }}</th>
                <th scope="col" class="num">{{ t("stats.finished") }}</th>
                <th scope="col" class="num">{{ t("stats.pages") }}</th>
                <th scope="col" class="num">{{ t("stats.averageShort") }}</th>
                <th scope="col" class="num">{{ t("stats.dnf") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in stats.members" :key="row.username">
                <th scope="row"><RouterLink :to="`/u/${row.username}/stats?year=${stats.year}`">{{ row.username }}</RouterLink></th>
                <td class="num">{{ formatCount(row.finished, locale.locale) }}</td>
                <td class="num">{{ formatCount(row.pages, locale.locale) }}</td>
                <td class="num">{{ formatRating(row.average_rating, locale.locale) }}</td>
                <td class="num">{{ formatCount(row.dnf, locale.locale) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="section" aria-labelledby="stats-picks">
        <div class="section-head">
          <h3 id="stats-picks">{{ t("stats.allPicks") }}</h3>
        </div>
        <p v-if="!stats.picks.length" class="fine subtle">{{ t("stats.noPicks") }}</p>
        <ol v-else class="pick-list">
          <li v-for="pick in stats.picks" :key="pick.pick_id">
            <RouterLink class="pick-row" :to="bookPath(pick.book)">
              <BookCover :book="pick.book" size="xs" />
              <span class="pick-text">
                <strong>{{ pick.book.title }}</strong>
                <span v-if="stats.best_pick?.pick_id === pick.pick_id" class="badge club-pick">{{ t("stats.bestPick") }}</span>
                <span class="finer subtle">{{ pickWhen(pick) }} · {{ t("stats.setBy", { name: pick.set_by }) }}</span>
                <span class="fine subtle">{{ pickLine(pick) }}</span>
              </span>
            </RouterLink>
          </li>
        </ol>
      </section>
    </template>
  </section>
</template>

<style scoped>
.stats-head {
  align-items: center;
}

.stats-skeleton {
  height: 260px;
}

.table-wrap {
  overflow-x: auto;
}

.member-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--text-sm);
}

.member-table th,
.member-table td {
  padding: var(--space-2);
  border-bottom: 1px solid var(--border);
  text-align: left;
  white-space: nowrap;
}

.member-table thead th {
  font-size: var(--text-2xs);
  font-weight: 700;
  color: var(--text-muted);
}

.member-table .num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.pick-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.pick-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  color: inherit;
  text-decoration: none;
}

.pick-text {
  display: grid;
  gap: 2px;
  min-width: 0;
  justify-items: start;
}
</style>
