<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import BookDiary from "../components/BookDiary.vue";
import MeetingSheet from "../components/MeetingSheet.vue";
import {
  bookPath,
  relativeDay,
  starLabel,
  statusLabel,
  statusShort,
} from "../constants";
import { formatClubDate } from "../i18n/dates";
import { tp } from "../i18n";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { ClubPick, NextUpVote } from "../types";

const { t } = useI18n();
const locale = useLocale();

const session = useSession();
const toast = useToast();

const pick = ref<ClubPick | null>(null);
const history = ref<ClubPick[]>([]);
const vote = ref<NextUpVote | null>(null);
const timezone = ref("UTC");
const error = ref("");
const loaded = ref(false);
const editingMeeting = ref(false);
const busy = ref(false);

const readers = computed(() =>
  [...(pick.value?.readers ?? [])].sort((a, b) => {
    const rank = { currently_reading: 0, finished: 1, want_to_read: 2, did_not_finish: 3 };
    return rank[a.status] - rank[b.status] || a.username.localeCompare(b.username);
  }),
);

const finishedCount = computed(
  () => readers.value.filter((row) => row.status === "finished").length,
);

const meetingCountdown = computed(() => relativeDay(pick.value?.meeting_at));

const leader = computed(() => {
  const nominations = vote.value?.nominations ?? [];
  if (nominations.length === 0) return null;
  return [...nominations].sort((a, b) => b.votes - a.votes)[0];
});

async function load() {
  try {
    const [current, past] = await Promise.all([api.clubPick(), api.clubPickHistory()]);
    pick.value = current.pick;
    timezone.value = current.timezone;
    history.value = past.items;
    error.value = "";
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("home.loadFailed");
  } finally {
    loaded.value = true;
  }
  try {
    vote.value = await api.nextUp();
  } catch {
    vote.value = null;
  }
}

async function startReading() {
  const current = pick.value;
  if (!current || busy.value) return;
  busy.value = true;
  try {
    const created = await api.addToShelf({
      ol_work_key: current.book.ol_work_key,
      title: current.book.title,
      authors: current.book.authors,
      cover_id: current.book.cover_id,
      cover_url: current.book.cover_url,
      year: current.book.year,
      status: "currently_reading",
    });
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

async function saveMeeting(meetingAt: string | null) {
  const current = pick.value;
  editingMeeting.value = false;
  if (!current) return;
  try {
    await api.setClubPick({
      ol_work_key: current.book.ol_work_key,
      title: current.book.title,
      authors: current.book.authors,
      cover_id: current.book.cover_id,
      cover_url: current.book.cover_url,
      year: current.book.year,
      meeting_at: meetingAt,
    });
    toast.show(meetingAt ? t("home.meetingSaved") : t("home.meetingCleared"));
    await load();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("home.meetingFailed"));
  }
}

onMounted(load);
</script>

<template>
  <section class="home">
    <div class="page-head">
      <h1>{{ session.user?.username ? t("home.hello", { name: session.user.username }) : t("home.title") }}</h1>
      <p class="lede">{{ t("home.lede") }}</p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <div v-if="!loaded" class="hero hero-skeleton" aria-hidden="true">
      <span class="skeleton hero-cover-skeleton" />
      <div class="hero-body">
        <span class="skeleton skeleton-line" />
        <span class="skeleton skeleton-line short" />
      </div>
    </div>

    <template v-else-if="pick">
      <article class="hero">
        <RouterLink class="hero-cover" :to="bookPath(pick.book.ol_work_key)">
          <BookCover
            :title="pick.book.title"
            :cover-id="pick.book.cover_id"
            :work-key="pick.book.ol_work_key"
            :image-url="pick.book.cover_url"
            eager
          />
        </RouterLink>
        <div class="hero-body">
          <p class="kicker">{{ t("home.readingNow") }}</p>
          <RouterLink class="hero-title" :to="bookPath(pick.book.ol_work_key)">
            <h2 class="display">{{ pick.book.title }}</h2>
          </RouterLink>
          <p v-if="pick.book.authors" class="hero-authors">{{ pick.book.authors }}</p>
          <p class="fine subtle">
            {{ t("home.chosenBy", { name: pick.set_by }) }}
            <template v-if="pick.book.year"> · {{ pick.book.year }}</template>
          </p>

          <p v-if="pick.meeting_at" class="meeting">
            <strong>{{ formatClubDate(pick.meeting_at, timezone, locale.locale) || pick.meeting_label }}</strong>
            <span v-if="meetingCountdown" class="subtle">{{ meetingCountdown }}</span>
          </p>
          <p v-else class="fine subtle">{{ t("home.noMeeting") }}</p>

          <p v-if="pick.note" class="note">{{ pick.note }}</p>

          <div class="btn-row hero-actions">
            <RouterLink
              class="btn btn-primary"
              :to="bookPath(pick.book.ol_work_key)"
            >
              {{ pick.on_shelf ? t("home.openBook") : t("home.seeDetails") }}
            </RouterLink>
            <button
              v-if="!pick.on_shelf"
              class="btn btn-ghost"
              type="button"
              :disabled="busy"
              @click="startReading"
            >
              {{ t("home.startReading") }}
            </button>
            <span v-else class="badge" :class="pick.on_shelf">
              {{ statusShort(pick.on_shelf) }}
            </span>
            <button
              class="btn btn-ghost"
              type="button"
              @click="editingMeeting = true"
            >
              {{ pick.meeting_at ? t("home.changeMeeting") : t("home.addMeeting") }}
            </button>
          </div>
        </div>
      </article>

      <section class="section progress-section" aria-labelledby="progress">
        <div class="section-head">
          <h2 id="progress">{{ t("home.whereEveryone") }}</h2>
          <span class="fine subtle nums">
            {{ t("home.finishedOf", { finished: finishedCount, total: readers.length || 0 }) }}
          </span>
        </div>
        <div v-if="readers.length" class="rail reader-rail">
          <RouterLink
            v-for="row in readers"
            :key="row.username"
            class="reader-chip"
            :to="`/club/${row.username}`"
          >
            <Avatar :username="row.username" :src="row.avatar_url" size="sm" />
            <span class="reader-name">{{ row.username }}</span>
            <span class="badge" :class="row.status">{{ statusShort(row.status) }}</span>
            <span v-if="row.progress != null" class="progress-track" aria-hidden="true">
              <span class="progress-fill" :style="{ width: `${row.progress}%` }" />
            </span>
            <span v-if="row.progress != null" class="finer subtle nums">
              {{ row.progress }}%
            </span>
            <span v-else-if="row.rating" class="finer stars">
              {{ starLabel(row.rating) }}
            </span>
            <span v-if="row.take" class="finer subtle clamp-2">{{ row.take }}</span>
          </RouterLink>
        </div>
        <p v-else class="fine subtle">
          {{ t("home.nobodyAdded") }}
        </p>
      </section>

      <section class="section thread-section">
        <BookDiary
          :work-key="pick.book.ol_work_key"
          :book="pick.book"
          :preview="3"
          :heading="t('home.diary')"
        />
      </section>
    </template>

    <div v-else class="empty">
      <h3>{{ t("home.noPickTitle") }}</h3>
      <p>{{ t("home.noPickBody") }}</p>
      <div class="btn-row">
        <RouterLink class="btn btn-primary" to="/discover">{{ t("home.findBook") }}</RouterLink>
        <RouterLink class="btn btn-ghost" to="/shelf">{{ t("home.pickFromShelf") }}</RouterLink>
      </div>
    </div>

    <section class="section next-up-section" aria-labelledby="next-up">
      <div class="section-head">
        <h2 id="next-up">{{ t("home.nextUp") }}</h2>
        <RouterLink to="/club">{{ t("home.voteInClub") }}</RouterLink>
      </div>
      <RouterLink v-if="leader" class="book-row next-up-card" to="/club">
        <BookCover
          :title="leader.book.title"
          :cover-id="leader.book.cover_id"
          :work-key="leader.book.ol_work_key"
          :image-url="leader.book.cover_url"
          size="sm"
        />
        <span class="book-row-meta">
          <h3>{{ leader.book.title }}</h3>
          <p class="fine">
            {{ tp("home.leadingVotes", leader.votes) }}
            <span class="subtle">
              · {{ t("home.nominatedCount", { n: vote?.nominations.length ?? 0 }) }}
            </span>
          </p>
        </span>
        <span class="fine">{{ t("home.voteArrow") }}</span>
      </RouterLink>
      <p v-else class="fine subtle">
        {{ t("home.noNominations") }}
      </p>
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
          :to="bookPath(row.book.ol_work_key)"
        >
          <BookCover
            :title="row.book.title"
            :cover-id="row.book.cover_id"
            :work-key="row.book.ol_work_key"
            :image-url="row.book.cover_url"
          />
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
  </section>
</template>

<style scoped>
.hero {
  display: grid;
  grid-template-columns: 128px 1fr;
  gap: var(--space-4);
  align-items: start;
}

.hero-skeleton {
  min-height: 200px;
}

.hero-cover-skeleton {
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-md);
}

.hero-cover {
  display: block;
  border-radius: var(--radius-sm);
}

.hero-body {
  min-width: 0;
  display: grid;
  gap: var(--space-1);
  align-content: start;
}

.hero-title {
  color: inherit;
  text-decoration: none;
}

.hero-body .display {
  font-size: var(--text-2xl);
}

.hero-authors {
  color: var(--text-muted);
  font-size: var(--text-lg);
}

.meeting {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-2);
  margin-top: var(--space-2);
}

.hero-actions {
  align-items: center;
  margin-top: var(--space-3);
}

.reader-rail {
  align-items: stretch;
}

.reader-chip {
  width: 150px;
  display: grid;
  gap: var(--space-1);
  align-content: start;
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: inherit;
  text-decoration: none;
}

.reader-chip:hover {
  border-color: var(--border-strong);
}

.reader-name {
  font-weight: 650;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.reader-chip :deep(.avatar) {
  margin-bottom: var(--space-1);
}

.reader-chip .badge {
  justify-self: start;
}

.progress-track {
  height: 5px;
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  overflow: hidden;
  margin-top: var(--space-1);
}

.progress-fill {
  display: block;
  height: 100%;
  background: var(--accent);
}

.next-up-card:hover {
  border-color: var(--border-strong);
}

.past-tile {
  width: 92px;
}

@media (min-width: 720px) {
  .hero {
    grid-template-columns: 208px 1fr;
    gap: var(--space-6);
  }

  .hero-body .display {
    font-size: var(--text-3xl);
  }
}

@media (min-width: 1024px) {
  .hero {
    grid-template-columns: 244px 1fr;
    align-items: center;
    padding: var(--space-6);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
  }

  /* Discussion is the long column; the vote card rides alongside it. Dense
     packing pulls the vote up beside the progress rail instead of leaving a
     hole where the hero's full-width row ends. */
  .home {
    display: grid;
    grid-template-columns: minmax(0, 1.7fr) minmax(0, 1fr);
    grid-auto-flow: row dense;
    column-gap: var(--space-6);
    align-items: start;
  }

  .home > .page-head,
  .home > .error,
  .home > .hero,
  .home > .empty,
  .home > .past-section {
    grid-column: 1 / -1;
  }

  .home > .progress-section,
  .home > .thread-section {
    grid-column: 1;
  }

  .home > .next-up-section {
    grid-column: 2;
  }
}
</style>
