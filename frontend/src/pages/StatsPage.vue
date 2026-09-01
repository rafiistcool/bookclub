<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import Skeleton from "../components/Skeleton.vue";
import StarRating from "../components/StarRating.vue";
import { useFlow, toRef } from "../stores/flow";
import type { Stats } from "../types";

const flow = useFlow();
const stats = ref<Stats | null>(null);
const error = ref("");
const year = ref<number | undefined>(undefined);

const monthMax = computed(() => Math.max(1, ...(stats.value?.by_month.map((m) => m.finished) ?? [1])));
const months = computed(() => {
  if (!stats.value) return [];
  const map = new Map(stats.value.by_month.map((m) => [m.month, m.finished]));
  return Array.from({ length: 12 }, (_, index) => {
    const key = `${stats.value!.year}-${String(index + 1).padStart(2, "0")}`;
    return { key, label: "JFMAMJJASOND"[index], finished: map.get(key) ?? 0 };
  });
});

async function load() {
  stats.value = null;
  try {
    stats.value = await api.stats(year.value);
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : "Could not load stats";
  }
}

function pickYear(value: number) {
  year.value = value;
  void load();
}

onMounted(load);
</script>

<template>
  <section aria-label="Year in review">
    <p v-if="error" class="error">{{ error }}</p>
    <Skeleton v-else-if="!stats" kind="lines" :count="6" />
    <template v-else>
      <div class="chip-row" role="group" aria-label="Year" style="margin-bottom: 14px">
        <button
          v-for="y in stats.years"
          :key="y"
          class="chip"
          type="button"
          :class="{ active: y === stats.year }"
          :aria-pressed="y === stats.year"
          @click="pickYear(y)"
        >
          {{ y }}
        </button>
      </div>

      <div class="stat-grid">
        <div class="stat"><div class="value">{{ stats.club_finished }}</div><div class="label">books finished</div></div>
        <div class="stat"><div class="value">{{ stats.club_pages.toLocaleString() }}</div><div class="label">pages</div></div>
        <div class="stat"><div class="value">{{ stats.club_average_rating ?? "–" }}</div><div class="label">average rating</div></div>
        <div class="stat"><div class="value">{{ stats.picks.filter((p) => new Date(p.started_at).getFullYear() === stats!.year).length }}</div><div class="label">club picks</div></div>
      </div>

      <section class="section" aria-labelledby="months-title">
        <div class="section-title"><h2 id="months-title">Finished per month</h2></div>
        <div class="bars" role="img" :aria-label="`Books finished per month in ${stats.year}`">
          <div v-for="m in months" :key="m.key" class="bar" :title="`${m.key}: ${m.finished}`">
            <span :style="{ height: `${(m.finished / monthMax) * 100}%` }" />
            <span>{{ m.label }}</span>
          </div>
        </div>
      </section>

      <section class="section" aria-labelledby="members-title">
        <div class="section-title"><h2 id="members-title">Members</h2></div>
        <div class="list">
          <article v-for="m in stats.members" :key="m.username" class="card">
            <div style="display: flex; align-items: center; gap: 10px">
              <Avatar :username="m.username" />
              <div style="flex: 1; min-width: 0">
                <RouterLink class="strong" :to="`/friends/${m.username}`" style="color: inherit; text-decoration: none">{{ m.username }}</RouterLink>
                <p class="fine muted">
                  {{ m.finished }} finished · {{ m.dnf }} DNF · {{ m.pages.toLocaleString() }} pages<template v-if="m.quotes"> · {{ m.quotes }} quotes</template>
                </p>
              </div>
              <div v-if="m.average_rating" class="fine" style="display: flex; gap: 4px; align-items: center">
                <StarRating :value="Math.round(m.average_rating)" /> {{ m.average_rating }}
              </div>
            </div>
            <div v-if="m.top_book || m.longest_book" class="actions" style="margin-top: 10px">
              <button v-if="m.top_book" class="chip" type="button" style="min-height: 44px; padding-left: 6px" @click="flow.open({ kind: 'details', book: toRef(m.top_book!) })">
                <BookCover :title="m.top_book.title" :cover-id="m.top_book.cover_id" size="xs" />
                <span class="clamp-1" style="max-width: 150px">Favourite: {{ m.top_book.title }}</span>
              </button>
              <button v-if="m.longest_book && m.longest_book.id !== m.top_book?.id" class="chip" type="button" style="min-height: 44px; padding-left: 6px" @click="flow.open({ kind: 'details', book: toRef(m.longest_book!) })">
                <BookCover :title="m.longest_book.title" :cover-id="m.longest_book.cover_id" size="xs" />
                <span class="clamp-1" style="max-width: 150px">Longest: {{ m.longest_book.title }}</span>
              </button>
            </div>
          </article>
        </div>
      </section>

      <section v-if="stats.picks.length" class="section" aria-labelledby="picks-title">
        <div class="section-title">
          <h2 id="picks-title">Club picks</h2>
          <span v-if="stats.best_pick" class="fine muted">Best: {{ stats.best_pick.book.title }}</span>
        </div>
        <ul class="list" style="list-style: none; padding: 0">
          <li v-for="p in stats.picks" :key="p.pick_id" class="row-item">
            <button type="button" style="all: unset; cursor: pointer" @click="flow.open({ kind: 'details', book: toRef(p.book) })">
              <BookCover :title="p.book.title" :cover-id="p.book.cover_id" size="sm" />
            </button>
            <div class="row-body">
              <h3 class="clamp-1">{{ p.book.title }}<span v-if="!p.ended_at" class="badge club" style="margin-left: 6px">Now</span></h3>
              <p class="clamp-1">{{ p.finished }} of {{ p.readers }} finished<template v-if="p.dnf"> · {{ p.dnf }} DNF</template> · {{ p.set_by }}</p>
            </div>
            <div v-if="p.average_rating" class="fine" style="display: grid; justify-items: end; gap: 2px">
              <StarRating :value="Math.round(p.average_rating)" />
              <span class="faint tiny">{{ p.average_rating }} · {{ p.ratings.length }}</span>
            </div>
          </li>
        </ul>
      </section>
    </template>
  </section>
</template>
