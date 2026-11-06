<script setup lang="ts">
import { CalendarDaysIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import Avatar from "../../components/Avatar.vue";
import BookCover from "../../components/BookCover.vue";
import BookDiary from "../../components/BookDiary.vue";
import MeetingSheet from "../../components/MeetingSheet.vue";
import MilestoneList from "../../components/MilestoneList.vue";
import ProgressEditor from "../../components/ProgressEditor.vue";
import ReaderRings from "../../components/ReaderRings.vue";
import { whenIdle } from "../../whenIdle";
import { bookPath, relativeDay, starLabel, statusLabel, statusShort } from "../../constants";
import { formatClubDate } from "../../i18n/dates";
import { tp } from "../../i18n";
import { isClubAdmin, isClubGone, useClubs, useCurrentClub } from "../../stores/clubs";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import type { ClubPick, NextUpVote, ShelfItem } from "../../types";

const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const clubs = useClubs();
const toast = useToast();

const club = useCurrentClub();
const slug = computed(() => club.value.slug);
const canManage = computed(() => isClubAdmin(club.value.role));

const pick = ref<ClubPick | null>(null);
const history = ref<ClubPick[]>([]);
const vote = ref<NextUpVote | null>(null);
const timezone = ref(club.value.timezone);
const error = ref("");
const loaded = ref(false);
const editingMeeting = ref(false);
const busy = ref(false);
const diary = ref<InstanceType<typeof BookDiary> | null>(null);

// My own row among the readers carries my progress for the club pick.
const me = computed(
  () => pick.value?.readers.find((row) => row.username === session.user?.username) ?? null,
);

const readers = computed(() =>
  [...(pick.value?.readers ?? [])].sort((a, b) => {
    const rank = { currently_reading: 0, finished: 1, want_to_read: 2, did_not_finish: 3 };
    return rank[a.status] - rank[b.status] || a.username.localeCompare(b.username);
  }),
);

const finishedCount = computed(
  () => readers.value.filter((row) => row.status === "finished").length,
);

/** Out of the whole club, not only those who have it on their shelf. */
const readerTotal = computed(() => Math.max(readers.value.length, club.value.member_count));

const ringReaders = computed(() =>
  readers.value.map((row) => ({
    username: row.username,
    avatar_url: row.avatar_url,
    progress: row.progress,
    status: row.status,
  })),
);

const takes = computed(() => readers.value.filter((row) => row.take));

// The countdown counts hours on the meeting's own day; a minute clock keeps it current.
const now = ref(Date.now());
let clock = 0;
onMounted(() => {
  clock = window.setInterval(() => (now.value = Date.now()), 60_000);
});
onUnmounted(() => window.clearInterval(clock));

const meetingCountdown = computed(() => {
  const at = pick.value?.meeting_at;
  const ahead = at ? new Date(at).getTime() - now.value : Number.NaN;
  if (ahead > 0 && ahead < 60 * 60_000) return t("home.inMinutes", { n: Math.max(1, Math.round(ahead / 60_000)) });
  if (ahead > 0 && ahead < 24 * 60 * 60_000) return tp("home.inHours", Math.round(ahead / (60 * 60_000)));
  return relativeDay(at, timezone.value);
});

const leader = computed(() => {
  const nominations = vote.value?.nominations ?? [];
  if (nominations.length === 0) return null;
  return [...nominations].sort((a, b) => b.votes - a.votes)[0];
});

function failed(err: unknown, fallback: string): string {
  if (isClubGone(err)) clubs.markGone(slug.value);
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  try {
    const [current, past] = await Promise.all([
      api.clubPick(slug.value),
      api.clubPickHistory(slug.value),
    ]);
    pick.value = current.pick;
    clubs.syncPick(slug.value, current.pick);
    timezone.value = current.timezone;
    history.value = past.items;
    error.value = "";
  } catch (err) {
    error.value = failed(err, t("home.loadFailed"));
  } finally {
    loaded.value = true;
  }
  try {
    vote.value = await api.vote(slug.value);
  } catch {
    vote.value = null;
  }
}

async function startReading() {
  const current = pick.value;
  if (!current || busy.value) return;
  busy.value = true;
  try {
    const created = await api.addToShelf({ work_id: current.book.id, status: "currently_reading" });
    current.on_shelf = created.status;
    current.shelf_id = created.id;
    toast.show(t("home.addedToReading"));
    await load();
  } catch (err) {
    if (err instanceof ApiError && err.status === 409 && err.item) {
      current.on_shelf = err.item.status;
      current.shelf_id = err.item.id;
      toast.show(t("home.alreadyOnShelf", { status: statusLabel(err.item.status) }));
    } else {
      toast.show(err instanceof ApiError ? err.message : t("home.addFailed"));
    }
  } finally {
    busy.value = false;
  }
}

// Only the newest progress save can be undone (an older toast's "before"
// would undo the newer save too).
let undoToast = 0;

async function saveProgress(value: number, undo = true) {
  // An Undo waits for a save still under way instead of being dropped.
  if (!undo) await whenIdle(busy);
  const current = pick.value;
  if (!current?.shelf_id || busy.value) return;
  if (!undo && current.on_shelf !== "currently_reading") return;
  const before = me.value?.progress ?? 0;
  busy.value = true;
  try {
    const item = await api.patchShelf(current.shelf_id, { progress: value });
    if (me.value) me.value.progress = item.progress;
    if (undoToast) toast.dismiss(undoToast);
    undoToast = 0;
    if (undo) {
      undoToast = toast.show(t("book.progressSaved", { n: value }), {
        label: t("common.undo"),
        run: () => saveProgress(before, false),
      });
    } else toast.show(t("progress.undone"));
    // Spoiler shields in the discussion depend on how far I am.
    void diary.value?.reload();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

/** ProgressEditor marked the book read (and opened the finished moment). */
function onFinished(item: ShelfItem) {
  const current = pick.value;
  if (!current) return;
  current.on_shelf = item.status;
  if (undoToast) toast.dismiss(undoToast);
  undoToast = 0;
  void load();
  void diary.value?.reload();
}

async function beginReading() {
  const current = pick.value;
  if (!current?.shelf_id || busy.value) return;
  busy.value = true;
  try {
    const item = await api.patchShelf(current.shelf_id, { status: "currently_reading", position: 0 });
    current.on_shelf = item.status;
    toast.show(t("home.addedToReading"));
    await load();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

/** The meeting edit: `PATCH …/pick` changes the meeting and nothing else. */
async function saveMeeting(meetingAt: string | null) {
  editingMeeting.value = false;
  if (!pick.value) return;
  try {
    pick.value = await api.patchClubPick(slug.value, { meeting_at: meetingAt });
    clubs.syncPick(slug.value, pick.value);
    toast.show(meetingAt ? t("home.meetingSaved") : t("home.meetingCleared"));
  } catch (err) {
    toast.show(failed(err, t("home.meetingFailed")));
  }
}

onMounted(load);
</script>

<template>
  <div class="club-home">
    <p v-if="error" class="error">{{ error }}</p>

    <div v-if="!loaded" class="hero hero-skeleton" aria-hidden="true">
      <span class="skeleton hero-cover-skeleton" />
      <span class="skeleton skeleton-line" />
      <span class="skeleton skeleton-line short" />
    </div>

    <template v-else-if="pick">
      <article class="hero" aria-labelledby="pick-title">
        <RouterLink class="hero-cover" :to="bookPath(pick.book)" tabindex="-1" aria-hidden="true">
          <BookCover :book="pick.book" eager />
        </RouterLink>
        <RouterLink class="hero-title" :to="bookPath(pick.book)">
          <h2 id="pick-title" class="display">{{ pick.book.title }}</h2>
        </RouterLink>
        <p v-if="pick.book.authors" class="hero-authors">{{ pick.book.authors }}</p>
        <p v-if="pick.set_by" class="fine subtle">{{ t("home.chosenBy", { name: pick.set_by }) }}</p>

        <div class="meeting">
          <p v-if="pick.meeting_at" class="meeting-when">
            <CalendarDaysIcon class="meeting-icon" aria-hidden="true" />
            <span>
              <strong v-if="meetingCountdown" class="countdown">{{ meetingCountdown }}</strong>
              <span class="meeting-date">
                {{ formatClubDate(pick.meeting_at, timezone, locale.locale) || pick.meeting_label }}
              </span>
            </span>
          </p>
          <p v-else class="fine subtle">{{ t("home.noMeeting") }}</p>
          <p class="meeting-links">
            <a v-if="pick.meeting_at" class="fine" :href="api.meetingIcsPath(slug)" download>
              {{ t("home.addToCalendar") }}
            </a>
            <button
              v-if="canManage"
              class="text-btn fine meeting-edit"
              type="button"
              @click="editingMeeting = true"
            >
              {{ pick.meeting_at ? t("home.changeMeeting") : t("home.addMeeting") }}
            </button>
          </p>
        </div>

        <!-- The viewer's one next step with this book. -->
        <div class="hero-action">
          <ProgressEditor
            v-if="pick.on_shelf === 'currently_reading' && pick.shelf_id"
            :value="me?.progress ?? 0"
            :pages="pick.book.pages"
            :label="t('home.yourProgress')"
            :book-title="pick.book.title"
            :busy="busy"
            :shelf-id="pick.shelf_id"
            :club-slug="slug"
            @save="saveProgress"
            @finished="onFinished"
          />
          <button
            v-else-if="!pick.on_shelf"
            class="btn btn-primary btn-block start-reading"
            type="button"
            :disabled="busy"
            @click="startReading"
          >
            {{ t("home.startReading") }}
          </button>
          <button
            v-else-if="pick.on_shelf === 'want_to_read' && pick.shelf_id"
            class="btn btn-primary btn-block start-reading"
            type="button"
            :disabled="busy"
            @click="beginReading"
          >
            {{ t("home.startReading") }}
          </button>
          <RouterLink
            v-else-if="pick.on_shelf === 'finished'"
            class="btn btn-ghost btn-block to-discussion"
            :to="`/c/${slug}/discussions`"
          >
            {{ t("home.toDiscussion") }}
          </RouterLink>
          <span v-else class="badge" :class="pick.on_shelf">{{ statusShort(pick.on_shelf) }}</span>
        </div>
        <p v-if="pick.note" class="note hero-note">{{ pick.note }}</p>
      </article>

      <section class="section progress-section" aria-labelledby="progress">
        <div class="section-head">
          <h2 id="progress">{{ t("home.whereEveryone") }}</h2>
          <span class="fine subtle nums">
            {{ t("home.finishedOf", { finished: finishedCount, total: readerTotal }) }}
          </span>
        </div>
        <ReaderRings
          v-if="readers.length"
          :readers="ringReaders"
          :member-count="club.member_count"
          size="md"
        />
        <p v-else class="fine subtle">{{ t("home.nobodyAdded") }}</p>
        <ul v-if="takes.length" class="reader-takes">
          <li v-for="row in takes" :key="row.username">
            <RouterLink class="take-who" :to="`/u/${row.username}`">
              <Avatar :username="row.username" :src="row.avatar_url" size="xs" />
              <strong>{{ row.username }}</strong>
            </RouterLink>
            <span v-if="row.rating" class="finer stars">{{ starLabel(row.rating) }}</span>
            <p>{{ row.take }}</p>
          </li>
        </ul>
      </section>

      <!-- Its own root is the section: a member sees none while there are no milestones. -->
      <MilestoneList
        class="section milestone-section"
        :slug="slug"
        :pick-id="pick.id"
        :can-manage="canManage"
        :timezone="timezone"
      />
    </template>

    <div v-else class="empty">
      <h3>{{ t("home.noPickTitle") }}</h3>
      <p>{{ canManage ? t("home.noPickBody") : t("home.noPickMemberBody") }}</p>
      <div class="btn-row">
        <RouterLink class="btn btn-primary" to="/discover">{{ t("home.findBook") }}</RouterLink>
        <RouterLink class="btn btn-ghost" to="/shelf">{{ t("home.pickFromShelf") }}</RouterLink>
      </div>
    </div>

    <section class="section next-up-section" aria-labelledby="next-up">
      <div class="section-head">
        <h2 id="next-up">{{ t("home.nextUp") }}</h2>
        <RouterLink :to="`/c/${slug}/vote`">{{ t("home.toVote") }}</RouterLink>
      </div>
      <RouterLink v-if="leader" class="next-up-card" :to="`/c/${slug}/vote`">
        <BookCover :book="leader.book" size="sm" />
        <span class="next-up-meta">
          <h3>{{ leader.book.title }}</h3>
          <p class="fine">
            {{ tp("home.leadingVotes", leader.votes) }}
            <span class="subtle">
              · {{ t("home.nominatedCount", { n: vote?.nominations.length ?? 0 }) }}
            </span>
          </p>
        </span>
      </RouterLink>
      <div v-else class="next-up-empty">
        <p class="fine subtle">{{ t("home.noNominations") }}</p>
        <div class="btn-row">
          <RouterLink class="btn btn-ghost btn-sm" :to="`/c/${slug}/overlap`">{{ t("home.seeOverlap") }}</RouterLink>
          <RouterLink class="btn btn-ghost btn-sm" to="/discover">{{ t("home.findBook") }}</RouterLink>
        </div>
      </div>
    </section>

    <section v-if="pick" class="section thread-section">
      <BookDiary
        ref="diary"
        :club="{ slug, pickId: null }"
        :preview="3"
        :heading="t('home.discussion')"
      />
      <RouterLink class="fine thread-link" :to="`/c/${slug}/discussions`">
        {{ t("home.wholeDiscussion") }}
      </RouterLink>
    </section>

    <section v-if="history.length" class="section past-section" aria-labelledby="past">
      <div class="section-head">
        <h2 id="past">{{ t("home.pastPicks") }}</h2>
        <span class="fine subtle nums">{{ history.length }}</span>
      </div>
      <div class="rail">
        <RouterLink
          v-for="row in history"
          :key="row.id"
          class="book-tile past-tile"
          :to="`/c/${slug}/discussions?pick=${row.id}`"
        >
          <BookCover :book="row.book" />
          <span class="book-tile-title">{{ row.book.title }}</span>
          <span class="book-tile-sub">{{ formatClubDate(row.meeting_at, timezone, locale.locale) || row.meeting_label || row.set_by }}</span>
        </RouterLink>
      </div>
    </section>

    <MeetingSheet
      v-if="editingMeeting && pick"
      :title="t('home.meetingTitle')"
      :book-title="pick.book.title"
      :timezone="timezone"
      :meeting-local="pick.meeting_local"
      :confirm-label="t('home.saveMeeting')"
      @confirm="saveMeeting"
      @close="editingMeeting = false"
    />
  </div>
</template>

<style scoped>
/* The pick stands in the middle of the page: a large cover, its title, the
   meeting, and the one thing the viewer does next. */
.hero {
  display: grid;
  justify-items: center;
  text-align: center;
  gap: var(--space-1);
}

.hero-skeleton {
  gap: var(--space-3);
}

.hero-skeleton .skeleton-line {
  width: 60%;
}

.hero-cover-skeleton {
  width: 128px;
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-md);
}

.hero-cover {
  display: block;
  width: 128px;
  margin-bottom: var(--space-3);
  border-radius: var(--radius-sm);
  transition: transform var(--dur-fast) var(--ease-out);
}

.hero-cover:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.hero-title {
  color: inherit;
  text-decoration: none;
  max-width: 32ch;
}

.hero .display {
  font-size: var(--text-2xl);
  overflow-wrap: anywhere;
}

.hero-authors {
  color: var(--text-muted);
  font-size: var(--text-lg);
}

.meeting {
  display: grid;
  justify-items: center;
  gap: 0;
  margin-top: var(--space-3);
}

.meeting-when {
  display: inline-flex;
  align-items: flex-start;
  gap: var(--space-2);
  text-align: left;
}

.meeting-when > span {
  display: grid;
}

.meeting-icon {
  flex: none;
  width: 20px;
  height: 20px;
  margin-top: 2px;
  color: var(--accent);
}

.countdown {
  font-weight: 700;
}

.meeting-date {
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.meeting-links {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  align-items: center;
  gap: 0 var(--space-4);
}

.meeting-links a {
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  font-weight: 600;
}

.meeting-edit {
  font-size: var(--text-sm);
}

.hero-note {
  max-width: 40ch;
  margin-top: var(--space-3);
  overflow-wrap: anywhere;
}

.hero-action {
  justify-self: stretch;
  display: grid;
  justify-items: stretch;
  width: 100%;
  max-width: 420px;
  margin: var(--space-3) auto 0;
  text-align: left;
}

.hero-action .badge {
  justify-self: center;
}

.next-up-empty {
  display: grid;
  gap: var(--space-2);
}

/* Takes as light rows under the rings, a hairline between them. */
.reader-takes {
  margin: var(--space-4) 0 0;
  padding: 0;
  list-style: none;
}

.reader-takes li {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1) var(--space-2);
  padding: var(--space-3) 0;
  border-top: 1px solid var(--border);
}

.take-who {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  min-height: 32px;
  color: inherit;
  text-decoration: none;
}

.reader-takes p {
  flex-basis: 100%;
  color: var(--text-muted);
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}

/* The leader of the vote as one light row. */
.next-up-card {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--space-3);
  align-items: center;
  padding: var(--space-2) 0;
  color: inherit;
  text-decoration: none;
  border-radius: var(--radius-sm);
  transition: transform var(--dur-fast) var(--ease-out);
}

.next-up-card:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.next-up-meta {
  min-width: 0;
}

.next-up-meta h3 {
  font-family: var(--serif);
  font-size: var(--text-md);
  font-weight: 650;
}

.next-up-meta p {
  color: var(--text-muted);
}

@media (hover: hover) {
  .next-up-card:hover h3 {
    color: var(--accent);
  }
}

.thread-link {
  display: inline-flex;
  align-items: center;
  min-height: var(--tap);
  margin-top: var(--space-2);
  font-weight: 600;
}

.past-tile {
  width: 92px;
}

@media (min-width: 720px) {
  .hero-cover,
  .hero-cover-skeleton {
    width: 200px;
  }

  .hero .display {
    font-size: var(--text-3xl);
  }
}

@media (min-width: 1024px) {
  /* Discussion is the long column; the vote rides alongside it. */
  .club-home {
    display: grid;
    grid-template-columns: minmax(0, 1.7fr) minmax(0, 1fr);
    grid-auto-flow: row dense;
    column-gap: var(--space-6);
    align-items: start;
  }

  .club-home > .error,
  .club-home > .hero,
  .club-home > .empty,
  .club-home > .past-section {
    grid-column: 1 / -1;
  }

  .club-home > .progress-section,
  .club-home > .thread-section {
    grid-column: 1;
  }

  .club-home > .next-up-section,
  .club-home > .milestone-section {
    grid-column: 2;
  }
}
</style>
