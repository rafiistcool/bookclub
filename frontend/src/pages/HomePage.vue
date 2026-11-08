<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import ActivityFeed from "../components/ActivityFeed.vue";
import BookCover from "../components/BookCover.vue";
import DiaryFeed from "../components/DiaryFeed.vue";
import HomeHero from "../components/HomeHero.vue";
import NavIcon from "../components/NavIcon.vue";
import ProgressEditor from "../components/ProgressEditor.vue";
import { bookPath } from "../constants";
import { positionOf, positionPatch, positionSaved, type ReadingPosition } from "../progress";
import { tp } from "../i18n";
import { heroClubSlug, pickClubSlug, useClubs } from "../stores/clubs";
import { useFinish } from "../stores/finish";
import { useFriends } from "../stores/friends";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { Favorite, ShelfItem } from "../types";

/**
 * Home: one book at the centre (`HomeHero`): the current pick of the hero
 * club (the default club, else the soonest meeting, else the first with a
 * book, else the newest join), with the other clubs as chips that swap it.
 * Below come the other books being read, a row to `/friends` only while
 * requests or invitations wait there (the shell loads them; Friends lives
 * under the profile, whose tab carries a dot meanwhile), and short previews of the diary and the activity feed (§4.4: it
 * carries friends' personal events, so it shows for a viewer with friends
 * and no club too). With no club (solo mode) Home makes no club call: the
 * hero is the viewer's top book being read, under one quiet line to create
 * or join a club.
 */
const { t } = useI18n();
const session = useSession();
const clubs = useClubs();
const friends = useFriends();
const finish = useFinish();
const toast = useToast();

const clubsLoaded = ref(false);
const clubsError = ref("");
/** The whole shelf: the hero's entry may be in any column. */
const items = ref<ShelfItem[]>([]);
const favorites = ref<Favorite[]>([]);
const shelfLoaded = ref(false);
const shelfError = ref("");
const progress = ref<Record<number, ReadingPosition>>({});
const savingId = ref<number | null>(null);
const diary = ref<InstanceType<typeof DiaryFeed> | null>(null);

const hasClubs = computed(() => clubs.hasClubs);
/** The home feed has something to show besides one's own events: club mates' or friends'. */
const showActivity = computed(() => hasClubs.value || (friends.lists?.friends.length ?? 0) > 0);

const reading = computed(() =>
  items.value
    .filter((item) => item.status === "currently_reading")
    .sort((a, b) => a.position - b.position),
);

/** The tie-break's choice, kept: a pick answer that moves a meeting does not move the hero away. */
const pinnedSlug = ref<string | null>(null);
const heroClub = computed(() => {
  if (!hasClubs.value || !clubsLoaded.value) return null;
  const find = (slug: string | null) => (slug ? clubs.list.find((club) => club.slug === slug) : undefined);
  return find(clubs.defaultSlug) ?? find(pinnedSlug.value) ?? find(heroClubSlug(clubs.list, clubs.lastSlug)) ?? null;
});
watch(heroClub, (club) => {
  pinnedSlug.value = club?.slug ?? null;
});
const otherClubs = computed(() => clubs.list.filter((club) => club.slug !== heroClub.value?.slug));

/** The viewer's entry for the hero book: the hero club's pick, or solo the top book being read. */
const heroEntry = computed<ShelfItem | null>(() => {
  if (!hasClubs.value) return reading.value[0] ?? null;
  const bookId = heroClub.value?.current_pick?.book.id;
  return bookId == null ? null : (items.value.find((item) => item.book.id === bookId) ?? null);
});
/** Solo: a hero only for a book being read. With clubs: always, as soon as the clubs are known. */
const showHero = computed(() =>
  hasClubs.value ? !clubsError.value && (!clubsLoaded.value || heroClub.value != null) : !shelfLoaded.value || heroEntry.value != null,
);
const heroPending = computed(() => (hasClubs.value ? !clubsLoaded.value : !shelfLoaded.value));

/** The books being read besides the hero's: one shelf entry never has two editors. */
const restReading = computed(() => reading.value.filter((item) => item.id !== heroEntry.value?.id));

// The previews page in place ("Show more"), with no "See all": no page holds
// Home's whole feed. /diary has only the viewer's own writing, a club's pages
// only that club and its members (not friends, not the viewer's own events
// from before they joined).

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
    items.value = shelf.items;
    favorites.value = shelf.favorites;
    progress.value = Object.fromEntries(
      shelf.items.filter((item) => item.status === "currently_reading").map((item) => [item.id, positionOf(item)]),
    );
    shelfError.value = "";
  } catch (err) {
    shelfError.value = err instanceof ApiError ? err.message : t("shelf.loadFailed");
  } finally {
    shelfLoaded.value = true;
  }
}

/**
 * An entry the hero, a row or the finished moment changed: Home's shelf takes
 * the server's answer. A new position or status reloads "Recently written",
 * whose spoiler guards come with the viewer's position (`my_progress`): a post
 * folded at the old position may be safe now, and an Undo to an earlier page
 * folds again what lies past it.
 */
function adopt(item: ShelfItem) {
  const at = items.value.findIndex((row) => row.id === item.id);
  const before = at < 0 ? null : items.value[at];
  items.value = at < 0 ? [item, ...items.value] : items.value.map((row) => (row.id === item.id ? item : row));
  if (item.status === "currently_reading") progress.value = { ...progress.value, [item.id]: positionOf(item) };
  if (!before || before.status !== item.status || before.progress !== item.progress) void diary.value?.reload();
}

// The moment a row's ProgressEditor opened saved the stars: the hero's "Rate it" goes.
watch(
  () => finish.lastSaved,
  (saved) => {
    if (saved && items.value.some((row) => row.id === saved.id)) adopt(saved);
  },
);

// Only the newest save of a book can be undone: an older toast's "before"
// would also undo the newer save.
const undoToasts = new Map<number, number>();

async function saveProgress(item: ShelfItem, value: ReadingPosition, undo = true) {
  const before = progress.value[item.id] ?? positionOf(item);
  progress.value = { ...progress.value, [item.id]: value };
  savingId.value = item.id;
  try {
    const updated = await api.patchShelf(item.id, positionPatch(value));
    adopt(updated);
    const previous = undoToasts.get(item.id);
    if (previous) toast.dismiss(previous);
    undoToasts.delete(item.id);
    if (undo) {
      const id = toast.show(positionSaved(positionOf(updated)), {
        label: t("common.undo"),
        run: () => saveProgress(updated, before, false),
      });
      undoToasts.set(item.id, id);
    } else toast.show(t("progress.undone"));
  } catch (err) {
    progress.value = { ...progress.value, [item.id]: before };
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    savingId.value = null;
  }
}

/** A row's ProgressEditor finished the book (and opened the moment): it leaves the list. */
function onRowFinished(item: ShelfItem) {
  const previous = undoToasts.get(item.id);
  if (previous) toast.dismiss(previous);
  undoToasts.delete(item.id);
  adopt(item);
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
    </div>

    <p v-if="hasClubs && clubsError" class="error">{{ clubsError }}</p>
    <HomeHero
      v-if="showHero"
      :club="heroClub"
      :others="otherClubs"
      :entry="heroEntry"
      :pending="heroPending"
      @entry="adopt"
    />

    <p v-if="!hasClubs" class="solo-prompt">
      <span class="subtle">{{ t("home.soloLine") }}</span>
      <RouterLink class="solo-link" to="/clubs/new">{{ t("clubs.create") }}</RouterLink>
      <span class="subtle" aria-hidden="true">·</span>
      <RouterLink class="solo-link" to="/clubs">{{ t("home.joinWithCode") }}</RouterLink>
    </p>

    <RouterLink v-if="friends.badge" class="friends-row" to="/friends">
      <NavIcon name="friends" :size="20" />
      <span class="friends-row-label">{{ tp("home.friendsWaiting", friends.badge) }}</span>
      <span class="friends-row-arrow" aria-hidden="true">→</span>
    </RouterLink>

    <section
      v-if="shelfError || !shelfLoaded || reading.length === 0 || restReading.length"
      class="section home-reading"
      aria-labelledby="reading-now"
    >
      <div class="section-head">
        <h2 id="reading-now">{{ t("home.currentlyReading") }}</h2>
        <RouterLink to="/shelf">{{ t("home.toShelf") }}</RouterLink>
      </div>
      <p v-if="shelfError" class="error">{{ shelfError }}</p>
      <ol v-else-if="!shelfLoaded" class="skeleton-rows reading-skeleton" aria-hidden="true">
        <li v-for="n in 2" :key="n" class="skeleton-row">
          <span class="skeleton reading-skeleton-cover" />
          <span class="skeleton-row-lines">
            <span class="skeleton skeleton-line" />
            <span class="skeleton skeleton-line short" />
          </span>
        </li>
      </ol>
      <div v-else-if="reading.length === 0" class="reading-empty">
        <p class="fine subtle">{{ t("home.nothingReading") }}</p>
        <RouterLink class="btn btn-ghost btn-sm" to="/discover">{{ t("home.findBook") }}</RouterLink>
      </div>
      <ul v-else class="reading-list">
        <li v-for="item in restReading" :key="item.id" class="reading-row">
          <RouterLink class="reading-cover" :to="bookPath(item.book)" tabindex="-1" aria-hidden="true">
            <BookCover :book="item.book" size="sm" />
          </RouterLink>
          <span class="reading-meta">
            <RouterLink class="reading-title" :to="bookPath(item.book)">{{ item.book.title }}</RouterLink>
            <span v-if="item.book.authors" class="finer subtle">{{ item.book.authors }}</span>
            <ProgressEditor
              :value="progress[item.id]?.progress ?? 0"
              :page="progress[item.id]?.page ?? null"
              :pages="item.book.pages"
              :label="t('home.yourProgress')"
              :book-title="item.book.title"
              :busy="savingId === item.id"
              :shelf-id="item.id"
              :club-slug="pickClubSlug(clubs.list, item.book.id)"
              @save="(value) => saveProgress(item, value)"
              @finished="onRowFinished"
            />
          </span>
        </li>
      </ul>
    </section>

    <div class="home-feeds">
      <section class="section">
        <DiaryFeed
          ref="diary"
          :heading="hasClubs ? t('home.recentlyWritten') : t('home.yourNotes')"
          :empty-text="hasClubs ? t('home.nothingWritten') : t('home.noNotes')"
          :limit="4"
        >
          <template #head>
            <RouterLink to="/diary">{{ t("home.toDiary") }}</RouterLink>
          </template>
        </DiaryFeed>
      </section>
      <section v-if="showActivity" class="section">
        <ActivityFeed :heading="t('activity.heading')" :limit="5" />
      </section>
      <section v-if="!hasClubs && favorites.length" class="section" aria-labelledby="favourites">
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
/* Loading: rows shaped like the reading rows, with their small covers. */
.reading-skeleton-cover {
  flex: none;
  width: 52px;
  height: 78px;
}

/* Quiet: one line under the hero, two links a finger can hit. */
.solo-prompt {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  column-gap: var(--space-2);
  margin-top: var(--space-3);
  font-size: var(--text-sm);
}

.solo-link {
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  font-weight: 600;
}

/* A light row between two hairlines, not a card. */
.friends-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-top: var(--space-5);
  padding: var(--space-2) 0;
  min-height: var(--tap);
  border-block: 1px solid var(--border);
  color: inherit;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.friends-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

@media (hover: hover) {
  .friends-row:hover .friends-row-arrow {
    color: var(--text);
  }
}

.friends-row :deep(svg) {
  color: var(--accent);
}

.friends-row-label {
  flex: 1;
  font-weight: 600;
}

.friends-row-arrow {
  color: var(--text-muted);
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
  column-gap: var(--space-5);
}

/* Light rows: a hairline between one book and the next. */
.reading-row {
  display: flex;
  gap: var(--space-3);
  padding: var(--space-3) 0;
}

.reading-row + .reading-row {
  border-top: 1px solid var(--border);
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

.favourite-tile {
  width: 92px;
}

@media (min-width: 720px) {
  .reading-list {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .reading-row + .reading-row {
    border-top: 0;
  }

  .reading-row:nth-child(n + 3) {
    border-top: 1px solid var(--border);
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
