<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import ActivityFeed from "../components/ActivityFeed.vue";
import BookCover from "../components/BookCover.vue";
import ClubCard from "../components/ClubCard.vue";
import DiaryFeed from "../components/DiaryFeed.vue";
import { bookPath } from "../constants";
import { useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { Favorite, ShelfItem } from "../types";

/**
 * Home: the viewer's clubs and what moves in them (the combined activity
 * feed and diary across every club), plus what they are reading. With no
 * club (solo mode) it is a personal page that makes no club call, with a way
 * to create or join a club.
 */
const { t } = useI18n();
const session = useSession();
const clubs = useClubs();
const toast = useToast();

const clubsLoaded = ref(false);
const clubsError = ref("");
const reading = ref<ShelfItem[]>([]);
const favorites = ref<Favorite[]>([]);
const shelfLoaded = ref(false);
const shelfError = ref("");
const progress = ref<Record<number, number>>({});
const savingId = ref<number | null>(null);

const hasClubs = computed(() => clubs.hasClubs);

async function loadClubs() {
  if (!clubs.hasClubs) {
    clubsLoaded.value = true;
    return;
  }
  try {
    await clubs.loadList();
    clubsError.value = "";
  } catch (err) {
    clubsError.value = err instanceof ApiError ? err.message : t("clubs.loadFailed");
  } finally {
    clubsLoaded.value = true;
  }
}

async function loadShelf() {
  try {
    const shelf = await api.myShelf();
    reading.value = shelf.items
      .filter((item) => item.status === "currently_reading")
      .sort((a, b) => a.position - b.position);
    favorites.value = shelf.favorites;
    progress.value = Object.fromEntries(reading.value.map((item) => [item.id, item.progress ?? 0]));
    shelfError.value = "";
  } catch (err) {
    shelfError.value = err instanceof ApiError ? err.message : t("shelf.loadFailed");
  } finally {
    shelfLoaded.value = true;
  }
}

async function saveProgress(item: ShelfItem) {
  const value = progress.value[item.id] ?? 0;
  savingId.value = item.id;
  try {
    const updated = await api.patchShelf(item.id, { progress: value });
    reading.value = reading.value.map((row) => (row.id === item.id ? updated : row));
    toast.show(t("book.progressSaved", { n: value }));
  } catch (err) {
    progress.value = { ...progress.value, [item.id]: item.progress ?? 0 };
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    savingId.value = null;
  }
}

onMounted(() => {
  void loadClubs();
  void loadShelf();
});
</script>

<template>
  <section class="home" :class="{ solo: !hasClubs }">
    <div class="page-head">
      <h1>{{ session.user?.username ? t("home.hello", { name: session.user.username }) : t("home.title") }}</h1>
      <p class="lede">{{ hasClubs ? t("home.lede") : t("home.soloLede") }}</p>
    </div>

    <section v-if="hasClubs" class="home-clubs" aria-labelledby="your-clubs">
      <div class="section-head">
        <h2 id="your-clubs">{{ t("home.yourClubs") }}</h2>
        <RouterLink to="/clubs">{{ t("home.allClubs") }}</RouterLink>
      </div>
      <p v-if="clubsError" class="error">{{ clubsError }}</p>
      <div v-else-if="!clubsLoaded" class="skeleton clubs-skeleton" aria-hidden="true" />
      <ul v-else class="club-grid">
        <li v-for="club in clubs.list" :key="club.slug">
          <ClubCard :club="club" />
        </li>
      </ul>
    </section>

    <section v-else class="panel solo-prompt" aria-labelledby="solo-prompt">
      <h2 id="solo-prompt">{{ t("home.soloTitle") }}</h2>
      <p class="fine muted">{{ t("home.soloBody") }}</p>
      <div class="btn-row">
        <RouterLink class="btn btn-primary btn-sm" to="/clubs/new">{{ t("clubs.create") }}</RouterLink>
        <RouterLink class="btn btn-ghost btn-sm" to="/clubs">{{ t("home.joinWithCode") }}</RouterLink>
      </div>
    </section>

    <section class="section home-reading" aria-labelledby="reading-now">
      <div class="section-head">
        <h2 id="reading-now">{{ t("home.currentlyReading") }}</h2>
        <RouterLink to="/shelf">{{ t("home.toShelf") }}</RouterLink>
      </div>
      <p v-if="shelfError" class="error">{{ shelfError }}</p>
      <div v-else-if="!shelfLoaded" class="skeleton reading-skeleton" aria-hidden="true" />
      <div v-else-if="reading.length === 0" class="reading-empty">
        <p class="fine subtle">{{ t("home.nothingReading") }}</p>
        <RouterLink class="btn btn-ghost btn-sm" to="/discover">{{ t("home.findBook") }}</RouterLink>
      </div>
      <ul v-else class="reading-list">
        <li v-for="item in reading" :key="item.id" class="reading-row">
          <RouterLink class="reading-cover" :to="bookPath(item.book)" tabindex="-1" aria-hidden="true">
            <BookCover :book="item.book" size="sm" />
          </RouterLink>
          <span class="reading-meta">
            <RouterLink class="reading-title" :to="bookPath(item.book)">{{ item.book.title }}</RouterLink>
            <span v-if="item.book.authors" class="finer subtle">{{ item.book.authors }}</span>
            <label class="progress-label" :for="`progress-${item.id}`">
              <span class="finer subtle">{{ t("home.yourProgress") }}</span>
              <strong class="nums fine">{{ progress[item.id] ?? 0 }}%</strong>
            </label>
            <input
              :id="`progress-${item.id}`"
              v-model.number="progress[item.id]"
              type="range"
              min="0"
              max="100"
              step="5"
              :disabled="savingId === item.id"
              @change="saveProgress(item)"
            />
          </span>
        </li>
      </ul>
    </section>

    <div class="home-feeds">
      <section class="section">
        <DiaryFeed
          :heading="hasClubs ? t('home.recentlyWritten') : t('home.yourNotes')"
          :empty-text="hasClubs ? t('home.nothingWritten') : t('home.noNotes')"
        >
          <template #head>
            <RouterLink to="/diary">{{ t("home.toDiary") }}</RouterLink>
          </template>
        </DiaryFeed>
      </section>
      <section v-if="hasClubs" class="section">
        <ActivityFeed :heading="t('activity.heading')" />
      </section>
      <section v-else-if="favorites.length" class="section" aria-labelledby="favourites">
        <div class="section-head">
          <h2 id="favourites">{{ t("home.favourites") }}</h2>
        </div>
        <div class="rail">
          <RouterLink
            v-for="row in favorites"
            :key="row.position"
            class="book-tile favourite-tile"
            :to="bookPath(row.book)"
          >
            <BookCover :book="row.book" />
            <span class="book-tile-title">{{ row.book.title }}</span>
          </RouterLink>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.clubs-skeleton,
.reading-skeleton {
  height: 110px;
}

.club-grid {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.solo-prompt {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.solo-prompt h2 {
  font-size: var(--text-lg);
}

.reading-empty {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.reading-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.reading-row {
  display: flex;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.reading-cover {
  flex: 0 0 auto;
}

.reading-meta {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
  align-content: start;
}

.reading-title {
  color: inherit;
  text-decoration: none;
  font-family: var(--serif);
  font-size: var(--text-md);
  font-weight: 650;
  line-height: var(--leading-snug);
}

.progress-label {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-2);
  margin-top: var(--space-1);
}

.reading-meta input[type="range"] {
  width: 100%;
  min-height: var(--tap);
  accent-color: var(--accent);
}

.favourite-tile {
  width: 92px;
}

@media (min-width: 720px) {
  .club-grid,
  .reading-list {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (min-width: 1024px) {
  .home-feeds {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
    column-gap: var(--space-6);
    align-items: start;
  }

  .home.solo .home-feeds {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }
}
</style>
