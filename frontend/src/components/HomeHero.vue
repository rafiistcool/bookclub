<script setup lang="ts">
import { computed, ref, shallowRef, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { bookPath, relativeDay, starLabel, statusLabel, statusShort, type Status } from "../constants";
import { formatClubDate } from "../i18n/dates";
import type { Locale } from "../i18n";
import { isClubGone, useClubs } from "../stores/clubs";
import { useFinish } from "../stores/finish";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { Book, ClubOut, ClubPick, ShelfItem } from "../types";
import { whenIdle } from "../whenIdle";
import BookCover from "./BookCover.vue";
import ProgressEditor from "./ProgressEditor.vue";
import ReaderRings, { type RingReader } from "./ReaderRings.vue";

/**
 * The centre of Home: one book, large. In a club it is the club's current
 * pick (one `GET …/pick`) with the meeting, everyone's rings and exactly one
 * action for the viewer: start it, move the progress (which finishes it at
 * the last page), or rate it. The viewer's other clubs sit below as chips
 * that swap the hero in place and become "the club" (`clubs.remember`).
 * Without a club (`club` null) it is the viewer's top book being read.
 *
 * Home owns the shelf: `entry` is the viewer's entry for the hero book (the
 * pick's work, or the solo book), and every change the hero makes comes back
 * as `entry` so the reading list and the rest of Home stay in step. A 404
 * "No such club" drops the club (`markGone`); Home then puts the next one here.
 */
const props = defineProps<{
  /** The hero club; null puts the viewer's own `entry` here. */
  club: ClubOut | null;
  /** The viewer's other clubs. */
  others?: ClubOut[];
  entry: ShelfItem | null;
  /** Home is still loading what the hero needs: its shape, without a call. */
  pending?: boolean;
}>();

const emit = defineEmits<{ entry: [item: ShelfItem] }>();

const { t } = useI18n();
const clubs = useClubs();
const session = useSession();
const locale = useLocale();
const toast = useToast();

type Shown = { club: ClubOut; pick: ClubPick | null; zone: string };

/** The club answer on screen. During a swap the old one stays until the new one is here. */
const shown = shallowRef<Shown | null>(null);
const error = ref("");
const busy = ref(false);
/** The cover settles in once, on arrival; a swap only cross-fades. */
const settle = ref(true);

let token = 0;

async function load(club: ClubOut) {
  const mine = ++token;
  error.value = "";
  try {
    const current = await api.clubPick(club.slug);
    if (mine !== token) return;
    shown.value = { club, pick: current.pick, zone: current.timezone };
    clubs.syncPick(club.slug, current.pick);
  } catch (err) {
    if (mine !== token) return;
    // Not (or no longer) a member: Home moves on to the next club.
    if (isClubGone(err)) {
      clubs.markGone(club.slug);
      return;
    }
    shown.value = null;
    error.value = err instanceof ApiError ? err.message : t("home.loadFailed");
  }
}

watch(
  () => props.club?.slug ?? null,
  (slug) => {
    if (slug && props.club) void load(props.club);
    else {
      token++;
      shown.value = null;
      error.value = "";
    }
  },
  { immediate: true },
);

const solo = computed(() => !props.club);
const loading = computed(() => props.pending || (!solo.value && !shown.value && !error.value));
const pick = computed(() => (solo.value ? null : shown.value?.pick ?? null));
const book = computed<Book | null>(() => (solo.value ? props.entry?.book ?? null : pick.value?.book ?? null));
const shownSlug = computed(() => shown.value?.club.slug ?? null);
const busySwap = computed(() => !solo.value && !!props.club && shownSlug.value !== props.club.slug);

/** Home's entry, when it is the entry of the book on screen (not yet of a club being swapped in). */
const entry = computed(() =>
  props.entry && book.value && props.entry.book.id === book.value.id ? props.entry : null,
);
const me = computed(
  () => pick.value?.readers.find((row) => row.username === session.user?.username) ?? null,
);
const status = computed<Status | null>(() => entry.value?.status ?? pick.value?.on_shelf ?? null);
const shelfId = computed(() => entry.value?.id ?? pick.value?.shelf_id ?? null);
const progress = computed(() => (entry.value ? entry.value.progress ?? 0 : me.value?.progress ?? 0));
const rating = computed(() => (entry.value ? entry.value.rating : me.value?.rating ?? null));

/** The pick's readers with my own row as Home knows it now (started, moved, finished here). */
const readers = computed<RingReader[]>(() => {
  const rows: RingReader[] = pick.value?.readers ?? [];
  const name = session.user?.username;
  if (!name) return rows;
  const mine = rows.find((row) => row.username === name);
  if (!status.value) return rows.filter((row) => row.username !== name);
  const row: RingReader = {
    username: name,
    avatar_url: mine?.avatar_url ?? session.user?.avatar_url ?? null,
    status: status.value,
    progress: status.value === "currently_reading" ? progress.value : null,
  };
  return mine ? rows.map((other) => (other.username === name ? row : other)) : [row, ...rows];
});

const BCP47: Record<Locale, string> = { en: "en-GB", de: "de-DE" };

/** "Do., 19. Nov." in the club's zone. */
function shortDay(iso: string, timeZone: string): string {
  try {
    return new Intl.DateTimeFormat(BCP47[locale.locale], {
      weekday: "short",
      day: "numeric",
      month: "short",
      timeZone: timeZone || "UTC",
    }).format(new Date(iso));
  } catch {
    return "";
  }
}

const meeting = computed(() => {
  const at = pick.value?.meeting_at;
  if (!at || !shown.value) return null;
  const zone = shown.value.zone || shown.value.club.timezone;
  return {
    at,
    text: t("home.meetingWhen", { when: relativeDay(at, zone), date: shortDay(at, zone) }),
    full: formatClubDate(at, zone, locale.locale),
  };
});

const otherClubs = computed(() => (props.others ?? []).filter((club) => club.slug !== props.club?.slug));

function swap(slug: string) {
  settle.value = false;
  clubs.remember(slug);
}

/**
 * An entry the hero changed: the pick on screen takes it, my reader row too,
 * so a fallback to the pick (during a swap, before Home's entry matches) is
 * never older than what was just saved.
 */
function adopt(item: ShelfItem) {
  const current = shown.value;
  if (current?.pick && current.pick.book.id === item.book.id) {
    const name = session.user?.username;
    const readers = current.pick.readers.map((row) =>
      row.username === name
        ? {
            ...row,
            status: item.status,
            progress: item.status === "currently_reading" ? (item.progress ?? 0) : null,
            rating: item.rating,
            take: item.take,
          }
        : row,
    );
    shown.value = { ...current, pick: { ...current.pick, on_shelf: item.status, shelf_id: item.id, readers } };
  }
  emit("entry", item);
}

async function startReading() {
  const target = book.value;
  if (!target || busy.value) return;
  busy.value = true;
  try {
    const id = shelfId.value;
    const item =
      id != null
        ? await api.patchShelf(id, { status: "currently_reading", position: 0 })
        : await api.addToShelf({ work_id: target.id, status: "currently_reading" });
    adopt(item);
    toast.show(t("home.addedToReading"));
  } catch (err) {
    if (err instanceof ApiError && err.status === 409 && err.item) {
      adopt(err.item);
      toast.show(t("home.alreadyOnShelf", { status: statusLabel(err.item.status) }));
    } else {
      toast.show(err instanceof ApiError ? err.message : t("home.addFailed"));
    }
  } finally {
    busy.value = false;
  }
}

// Only the newest save can be undone: an older toast's "before" would undo the newer one too.
let undoToast = 0;

/**
 * `target` is the entry the save is for, taken when it is made: an Undo
 * tapped after a chip swap still restores the book it belongs to, not the
 * one the hero shows by then.
 */
async function saveProgress(value: number, undo = true, target: number | null = shelfId.value) {
  // An Undo waits for a save still under way instead of being dropped.
  if (!undo) await whenIdle(busy);
  const id = target;
  if (id == null || busy.value) return;
  // The status is known only while the hero still shows that entry.
  if (!undo && id === shelfId.value && status.value !== "currently_reading") return;
  const before = progress.value;
  busy.value = true;
  try {
    const item = await api.patchShelf(id, { progress: value });
    adopt(item);
    if (undoToast) toast.dismiss(undoToast);
    undoToast = 0;
    if (undo) {
      undoToast = toast.show(t("book.progressSaved", { n: value }), {
        label: t("common.undo"),
        run: () => saveProgress(before, false, id),
      });
    } else toast.show(t("progress.undone"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

/** The ProgressEditor finished the book and opened the moment itself. */
function onFinished(item: ShelfItem) {
  if (undoToast) toast.dismiss(undoToast);
  undoToast = 0;
  adopt(item);
}

async function rate() {
  const item = entry.value;
  if (!item) return;
  const saved = await useFinish().celebrate({ item, clubSlug: shownSlug.value });
  if (saved) adopt(saved);
}

const blankInitial = computed(() => shown.value?.club.name.charAt(0).toUpperCase() ?? "");
const swapKey = computed(() => (solo.value ? `solo-${props.entry?.id ?? 0}` : `club-${shownSlug.value}`));
</script>

<template>
  <section class="home-hero" :aria-busy="loading || busySwap || undefined" :aria-label="shown?.club.name ?? book?.title">
    <div v-if="loading" class="hero-skeleton" aria-hidden="true">
      <span class="skeleton hero-cover-skeleton" />
      <span class="hero-text">
        <span class="skeleton skeleton-line" />
        <span class="skeleton skeleton-line" />
        <span class="skeleton skeleton-line short" />
      </span>
    </div>

    <p v-else-if="error" class="error hero-error">
      <template v-if="club">{{ club.name }}: </template>{{ error }}
    </p>

    <Transition v-else name="hero-swap" mode="out-in">
      <div :key="swapKey" class="hero-swap">
        <RouterLink v-if="!solo && shown" class="hero-club" :to="`/c/${shown.club.slug}`">
          {{ shown.club.name }}<span aria-hidden="true">→</span>
        </RouterLink>

        <template v-if="book">
          <div class="hero-main">
            <RouterLink
              class="hero-cover"
              :class="{ settle }"
              :to="bookPath(book)"
              tabindex="-1"
              aria-hidden="true"
            >
              <BookCover :book="book" eager />
            </RouterLink>
            <div class="hero-text">
              <h2 class="hero-title">
                <RouterLink :to="bookPath(book)">{{ book.title }}</RouterLink>
              </h2>
              <p v-if="book.authors" class="hero-authors">{{ book.authors }}</p>
              <p v-if="meeting" class="hero-meeting fine">
                <time :datetime="meeting.at" :title="meeting.full">{{ meeting.text }}</time>
              </p>
            </div>
          </div>

          <ReaderRings
            v-if="!solo && shown"
            class="hero-rings"
            :readers="readers"
            :member-count="shown.club.member_count"
            size="md"
            :max="5"
          />

          <!-- During a swap the outgoing book has no action: Home's list may already hold its editor. -->
          <div v-if="!busySwap" class="hero-action">
            <ProgressEditor
              v-if="status === 'currently_reading' && shelfId != null"
              :value="progress"
              :pages="book.pages"
              :label="t('home.yourProgress')"
              :book-title="book.title"
              :busy="busy"
              :shelf-id="shelfId"
              :club-slug="shownSlug"
              @save="(value) => saveProgress(value)"
              @finished="onFinished"
            />
            <button
              v-else-if="!status || status === 'want_to_read'"
              class="btn btn-primary hero-start"
              type="button"
              :disabled="busy"
              @click="startReading"
            >
              {{ t("home.start") }}
            </button>
            <template v-else-if="status === 'finished' && !rating">
              <button v-if="entry" class="btn btn-primary hero-rate" type="button" @click="rate">
                {{ t("home.rate") }}
              </button>
              <RouterLink v-else class="btn btn-primary hero-rate" :to="bookPath(book)">
                {{ t("home.rate") }}
              </RouterLink>
            </template>
            <p v-else class="hero-done">
              <span v-if="status === 'finished' && rating" class="stars" :aria-label="t('book.starsOf', { n: rating })">
                {{ starLabel(rating) }}
              </span>
              <span v-else-if="status" class="badge" :class="status">{{ statusShort(status) }}</span>
              <RouterLink class="text-btn hero-open" :to="bookPath(book)">{{ t("home.openBook") }}</RouterLink>
            </p>
          </div>
        </template>

        <template v-else-if="shown">
          <div class="hero-main">
            <span class="hero-blank" :class="{ settle }" aria-hidden="true">{{ blankInitial }}</span>
            <div class="hero-text">
              <h2 class="hero-title">{{ t("home.noBookYet") }}</h2>
            </div>
          </div>
          <div class="hero-action">
            <RouterLink class="btn btn-primary hero-vote" :to="`/c/${shown.club.slug}/vote`">
              {{ t("home.toVote") }}
            </RouterLink>
          </div>
        </template>
      </div>
    </Transition>

    <div v-if="!solo && otherClubs.length" class="hero-others" role="group" :aria-label="t('home.otherClubs')">
      <span class="fine subtle" aria-hidden="true">{{ t("home.alsoIn") }}</span>
      <button
        v-for="other in otherClubs"
        :key="other.slug"
        class="chip hero-chip"
        type="button"
        :title="other.name"
        @click="swap(other.slug)"
      >
        <span class="hero-chip-name">{{ other.name }}</span>
      </button>
      <RouterLink class="hero-all fine" to="/clubs">{{ t("home.allClubs") }}</RouterLink>
    </div>
  </section>
</template>

<style scoped>
/* One column the width of the page: a long chip shrinks inside it instead of widening it. */
.home-hero {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: var(--space-4);
}

.hero-swap {
  display: grid;
  gap: var(--space-4);
}

.hero-club {
  justify-self: start;
  display: inline-flex;
  align-items: center;
  /* A flex item drops its leading space, so the arrow keeps its gap here. */
  gap: 0.3em;
  min-height: var(--tap);
  margin: calc(-1 * var(--space-3)) 0 calc(-1 * var(--space-2));
  color: var(--text-muted);
  font-weight: 600;
  text-decoration: none;
}

@media (hover: hover) {
  .hero-club:hover {
    color: var(--text);
  }
}

.hero-main,
.hero-skeleton {
  display: grid;
  grid-template-columns: 124px minmax(0, 1fr);
  gap: var(--space-4);
  align-items: end;
}

.hero-cover,
.hero-blank,
.hero-cover-skeleton {
  display: block;
  width: 100%;
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-sm);
}

.hero-cover {
  box-shadow: var(--elev-float);
}

.hero-cover :deep(.cover) {
  display: grid;
  width: 100%;
}

.hero-blank {
  display: grid;
  place-items: center;
  background: var(--accent-soft);
  color: var(--accent-hover);
  font-family: var(--serif);
  font-size: var(--text-4xl);
  font-weight: 700;
}

/* The page's one authored moment: the book is set down and lifts off the paper. */
.settle {
  animation: hero-settle calc(var(--dur-slow) * 2) var(--ease-out) both;
}

@keyframes hero-settle {
  from {
    transform: translateY(10px) scale(0.97);
    box-shadow: 0 0 0 transparent;
  }
}

.hero-text {
  display: grid;
  gap: var(--space-1);
  align-content: end;
  min-width: 0;
  padding-bottom: var(--space-1);
}

.hero-title {
  font-family: var(--serif);
  font-size: var(--text-2xl);
  line-height: var(--leading-tight);
  letter-spacing: var(--tracking-tight);
  overflow-wrap: anywhere;
  hyphens: auto;
}

.hero-title a {
  color: inherit;
  text-decoration: none;
}

.hero-authors {
  color: var(--text-muted);
  overflow-wrap: anywhere;
}

.hero-meeting {
  margin-top: var(--space-1);
  color: var(--accent-hover);
  font-weight: 600;
}

.hero-action {
  display: grid;
}

.hero-action .btn {
  width: 100%;
}

.hero-done {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.hero-done .text-btn {
  margin-left: auto;
  text-decoration: none;
}

.hero-error {
  margin: 0;
}

.hero-others {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1) var(--space-2);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}

/* A 36px pill with a finger-sized hit area around it. A long club name
   (up to 80 characters) ends in an ellipsis instead of widening the page. */
.hero-chip {
  flex-shrink: 1;
  min-width: 0;
  max-width: 100%;
  min-height: 36px;
}

.hero-chip-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hero-chip::before {
  inset: -4px -2px;
}

.hero-all {
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  margin-left: auto;
  font-weight: 600;
}

.hero-swap-enter-active {
  transition: opacity var(--dur-base) var(--ease-out);
}

.hero-swap-leave-active {
  transition: opacity var(--dur-fast) var(--ease-in);
}

.hero-swap-enter-from,
.hero-swap-leave-to {
  opacity: 0;
}

@media (min-width: 720px) {
  .hero-main,
  .hero-skeleton {
    grid-template-columns: 164px minmax(0, 1fr);
    gap: var(--space-5);
  }

  .hero-title {
    font-size: var(--text-3xl);
  }

  .hero-action {
    max-width: 420px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .settle {
    animation: none;
  }
}
</style>
