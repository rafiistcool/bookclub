<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import BookCover from "../components/BookCover.vue";
import BookDiary from "../components/BookDiary.vue";
import MeetingSheet from "../components/MeetingSheet.vue";
import {
  openLibraryUrl,
  STATUSES,
  STATUS_LABEL,
  STATUS_SHORT,
  starLabel,
  type Status,
} from "../constants";
import { useToast } from "../stores/toast";
import type { BookDetail } from "../types";

const route = useRoute();
const toast = useToast();

const book = ref<BookDetail | null>(null);
const error = ref("");
const loaded = ref(false);
const busy = ref(false);
const refreshing = ref(false);
const expanded = ref(false);
const settingPick = ref(false);
const clubTimezone = ref("UTC");

const takeDraft = ref("");
const reasonDraft = ref("");
const progressDraft = ref(0);
const diary = ref<InstanceType<typeof BookDiary> | null>(null);

const workId = computed(() => String(route.params.workId || ""));
const longDescription = computed(() => (book.value?.description.length ?? 0) > 420);

function syncDrafts(detail: BookDetail) {
  takeDraft.value = detail.take;
  reasonDraft.value = detail.dnf_reason;
  progressDraft.value = detail.progress ?? 0;
}

async function load() {
  loaded.value = false;
  error.value = "";
  expanded.value = false;
  try {
    const detail = await api.book(workId.value);
    book.value = detail;
    syncDrafts(detail);
  } catch (err) {
    book.value = null;
    error.value = err instanceof ApiError ? err.message : "Could not load that book";
  } finally {
    loaded.value = true;
  }
  try {
    clubTimezone.value = (await api.clubPick()).timezone;
  } catch {
    /* The timezone is only a label on the meeting field. */
  }
}

async function setStatus(status: Status) {
  const detail = book.value;
  if (!detail || busy.value) return;
  busy.value = true;
  try {
    if (detail.shelf_id) {
      await api.patchShelf(detail.shelf_id, { status, position: 0 });
    } else {
      const created = await api.addToShelf({
        ol_work_key: detail.ol_work_key,
        title: detail.title,
        authors: detail.authors,
        cover_id: detail.cover_id,
        year: detail.year,
        status,
      });
      detail.shelf_id = created.id;
    }
    detail.on_shelf = status;
    toast.show(`Moved to ${STATUS_LABEL[status]}`);
  } catch (err) {
    // Another tab or the importer may have shelved it since this page loaded.
    if (err instanceof ApiError && err.status === 409 && err.item) {
      detail.shelf_id = err.item.id;
      detail.on_shelf = err.item.status;
      busy.value = false;
      await setStatus(status);
      return;
    }
    toast.show(err instanceof ApiError ? err.message : "Could not save that");
  } finally {
    busy.value = false;
  }
}

async function patch(
  body: Parameters<typeof api.patchShelf>[1],
  message: string,
  action?: { label: string; run: () => void | Promise<void> },
) {
  const detail = book.value;
  if (!detail?.shelf_id || busy.value) return;
  busy.value = true;
  try {
    const item = await api.patchShelf(detail.shelf_id, body);
    detail.rating = item.rating;
    detail.take = item.take;
    detail.dnf_reason = item.dnf_reason;
    detail.progress = item.progress;
    syncDrafts(detail);
    toast.show(message, action);
    if ("rating" in body) void refreshClubRating();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not save that");
  } finally {
    busy.value = false;
  }
}

/** The club average moved with my stars; pull it without disturbing the page. */
async function refreshClubRating() {
  const detail = book.value;
  if (!detail) return;
  try {
    const fresh = await api.book(workId.value);
    detail.club_rating = fresh.club_rating;
    detail.rating_count = fresh.rating_count;
    detail.readers = fresh.readers;
  } catch {
    /* Cosmetic; the next load catches up. */
  }
}

function rate(value: number) {
  const detail = book.value;
  if (!detail) return;
  if (detail.rating === value) {
    void patch({ rating: null }, "Rating cleared");
    return;
  }
  void patch({ rating: value }, `Rated ${value} of 5`, {
    label: "Write a few lines",
    run: () => diary.value?.focusComposer(),
  });
}

async function remove() {
  const detail = book.value;
  if (!detail?.shelf_id) return;
  const previous = {
    status: detail.on_shelf as Status,
    rating: detail.rating,
    take: detail.take,
    dnf_reason: detail.dnf_reason,
    progress: detail.progress,
  };
  busy.value = true;
  try {
    await api.removeFromShelf(detail.shelf_id);
    detail.shelf_id = null;
    detail.on_shelf = null;
    detail.rating = null;
    detail.take = "";
    detail.dnf_reason = "";
    detail.progress = null;
    syncDrafts(detail);
    toast.show("Removed from your shelf", {
      label: "Undo",
      run: async () => {
        const created = await api.addToShelf({
          ol_work_key: detail.ol_work_key,
          title: detail.title,
          authors: detail.authors,
          cover_id: detail.cover_id,
          year: detail.year,
          ...previous,
        });
        detail.shelf_id = created.id;
        detail.on_shelf = created.status;
        detail.rating = created.rating;
        detail.take = created.take;
        detail.dnf_reason = created.dnf_reason;
        detail.progress = created.progress;
        syncDrafts(detail);
      },
    });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not remove that book");
  } finally {
    busy.value = false;
  }
}

async function confirmClubPick(meetingAt: string | null) {
  const detail = book.value;
  settingPick.value = false;
  if (!detail) return;
  try {
    await api.setClubPick({
      ol_work_key: detail.ol_work_key,
      title: detail.title,
      authors: detail.authors,
      cover_id: detail.cover_id,
      year: detail.year,
      meeting_at: meetingAt,
    });
    detail.club_pick = true;
    toast.show("Set as the club pick");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not set the club pick");
  }
}

async function refreshDetails() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    const detail = await api.refreshBook(workId.value);
    book.value = detail;
    syncDrafts(detail);
    toast.show("Details updated from Open Library");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not refresh that book");
  } finally {
    refreshing.value = false;
  }
}

async function nominate() {
  const detail = book.value;
  if (!detail) return;
  try {
    await api.nominate({
      ol_work_key: detail.ol_work_key,
      title: detail.title,
      authors: detail.authors,
      cover_id: detail.cover_id,
      year: detail.year,
    });
    toast.show("Nominated for the next-up vote");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : "Could not nominate that book");
  }
}

onMounted(load);
watch(workId, load);
</script>

<template>
  <section>
    <p class="fine back-link">
      <RouterLink to="/discover">← Discover</RouterLink>
    </p>

    <div v-if="!loaded" class="detail-hero" aria-hidden="true">
      <span class="skeleton hero-cover-skeleton" />
      <div class="detail-intro">
        <span class="skeleton skeleton-line" />
        <span class="skeleton skeleton-line short" />
      </div>
    </div>

    <div v-else-if="error" class="empty">
      <h3>Could not load that book</h3>
      <p>{{ error }}</p>
      <div class="btn-row">
        <button class="btn btn-ghost" type="button" @click="load">Try again</button>
        <RouterLink class="btn btn-ghost" to="/discover">Back to Discover</RouterLink>
      </div>
    </div>

    <template v-else-if="book">
      <div class="detail-hero">
        <div class="hero-cover">
          <BookCover :title="book.title" :cover-id="book.cover_id" />
        </div>
        <div class="detail-side">
          <div class="detail-intro">
            <p v-if="book.club_pick" class="kicker">Current club pick</p>
            <h1 class="display">{{ book.title }}</h1>
            <p v-if="book.authors" class="detail-authors">{{ book.authors }}</p>
            <p v-if="book.rating_count" class="club-rating">
              <span class="stars" aria-hidden="true">★</span>
              <strong class="nums">{{ book.club_rating?.toFixed(1) }}</strong>
              <span class="fine subtle">
                club rating · {{ book.rating_count }}
                {{ book.rating_count === 1 ? "rating" : "ratings" }}
              </span>
            </p>
            <p v-if="book.year" class="fine subtle nums">First published {{ book.year }}</p>
            <p class="fine">
              <a :href="openLibraryUrl(book.ol_work_key)" target="_blank" rel="noreferrer">
                View on Open Library
              </a>
              <span class="subtle"> · </span>
              <button
                class="text-btn"
                type="button"
                :disabled="refreshing"
                @click="refreshDetails"
              >
                {{ refreshing ? "Refreshing…" : "Refresh details" }}
              </button>
            </p>
          </div>

          <div class="panel shelf-panel">
            <p class="caps subtle">Your shelf</p>
            <div class="segmented status-control" role="group" aria-label="Shelf status">
              <button
                v-for="status in STATUSES"
                :key="status"
                type="button"
                :aria-pressed="book.on_shelf === status"
                :disabled="busy"
                @click="setStatus(status)"
              >
                {{ STATUS_SHORT[status] }}
              </button>
            </div>

            <p v-if="!book.on_shelf" class="fine subtle">
              Not on your shelf yet. Pick a status to add it.
            </p>

            <div v-if="book.on_shelf === 'currently_reading'" class="control-block">
              <label class="progress-label" for="progress">
                <span>Progress</span>
                <strong class="nums">{{ progressDraft }}%</strong>
              </label>
              <input
                id="progress"
                v-model.number="progressDraft"
                type="range"
                min="0"
                max="100"
                step="5"
                :disabled="busy"
                @change="patch({ progress: progressDraft }, `Progress: ${progressDraft}%`)"
              />
            </div>

            <div v-if="book.on_shelf === 'finished'" class="control-block">
              <span class="fine">Your rating</span>
              <div class="star-row">
                <button
                  v-for="value in 5"
                  :key="value"
                  class="star-btn"
                  type="button"
                  :aria-label="`${value} of 5`"
                  :aria-pressed="book.rating === value"
                  :disabled="busy"
                  @click="rate(value)"
                >
                  {{ book.rating && book.rating >= value ? "★" : "☆" }}
                </button>
              </div>
              <label class="field">
                <span>One-line take</span>
                <input
                  v-model="takeDraft"
                  type="text"
                  maxlength="140"
                  placeholder="What stood out?"
                />
              </label>
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busy || takeDraft === book.take"
                @click="patch({ take: takeDraft }, 'Take saved')"
              >
                Save take
              </button>
            </div>

            <div v-if="book.on_shelf === 'did_not_finish'" class="control-block">
              <label class="field">
                <span>Why didn't you finish?</span>
                <input
                  v-model="reasonDraft"
                  type="text"
                  maxlength="200"
                  placeholder="Optional"
                />
              </label>
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busy || reasonDraft === book.dnf_reason"
                @click="patch({ dnf_reason: reasonDraft }, 'Reason saved')"
              >
                Save reason
              </button>
            </div>

            <div class="btn-row panel-actions">
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="book.club_pick"
                @click="settingPick = true"
              >
                {{ book.club_pick ? "Already the club pick" : "Set as club pick" }}
              </button>
              <button class="btn btn-ghost btn-sm" type="button" @click="nominate">
                Nominate for next up
              </button>
              <button
                v-if="book.on_shelf"
                class="btn btn-danger btn-sm"
                type="button"
                :disabled="busy"
                @click="remove"
              >
                Remove
              </button>
            </div>
          </div>
        </div>
      </div>

      <section v-if="book.description" class="section">
        <div class="section-head">
          <h2>About</h2>
        </div>
        <p class="description" :class="{ clamped: longDescription && !expanded }">
          {{ book.description }}
        </p>
        <button
          v-if="longDescription"
          class="text-btn"
          type="button"
          @click="expanded = !expanded"
        >
          {{ expanded ? "Show less" : "Read more" }}
        </button>
      </section>

      <section class="section">
        <BookDiary
          ref="diary"
          :work-key="book.ol_work_key"
          :book="{
            title: book.title,
            authors: book.authors,
            cover_id: book.cover_id,
            year: book.year,
          }"
        />
      </section>

      <section v-if="book.subjects.length" class="section">
        <div class="section-head">
          <h2>Subjects</h2>
        </div>
        <div class="chip-row">
          <RouterLink
            v-for="subject in book.subjects"
            :key="subject"
            class="chip"
            :to="{ path: '/discover', query: { q: subject } }"
          >
            {{ subject }}
          </RouterLink>
        </div>
      </section>

      <section class="section">
        <div class="section-head">
          <h2>Other readers</h2>
        </div>
        <ul v-if="book.readers.length" class="reader-list">
          <li v-for="reader in book.readers" :key="reader.username">
            <RouterLink :to="`/club/${reader.username}`">{{ reader.username }}</RouterLink>
            <span class="badge" :class="reader.status">
              {{ STATUS_SHORT[reader.status] }}
            </span>
            <span v-if="reader.progress != null" class="fine subtle nums">
              {{ reader.progress }}%
            </span>
            <span v-if="reader.rating" class="fine stars">
              {{ starLabel(reader.rating) }}
            </span>
          </li>
        </ul>
        <p v-else class="fine subtle">Nobody else in the club has this one yet.</p>
      </section>
    </template>

    <MeetingSheet
      v-if="settingPick && book"
      title="Set as club pick"
      :book-title="book.title"
      :timezone="clubTimezone"
      blurb="This replaces the current pick for everyone."
      @confirm="confirmClubPick"
      @close="settingPick = false"
    />
  </section>
</template>

<style scoped>
.back-link {
  margin-bottom: var(--space-4);
}

.detail-hero {
  display: grid;
  grid-template-columns: 120px 1fr;
  gap: var(--space-4);
  align-items: start;
  margin-bottom: var(--space-5);
}

/* Narrow screens put the intro beside the cover and the shelf panel on its own
   full-width row, so the wrapper dissolves into the hero grid. */
.detail-side {
  display: contents;
}

.detail-hero .shelf-panel {
  grid-column: 1 / -1;
}

.hero-cover-skeleton {
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-md);
}

.detail-intro {
  min-width: 0;
  display: grid;
  gap: var(--space-1);
  align-content: start;
}

.detail-intro .display {
  font-size: var(--text-2xl);
}

.detail-authors {
  color: var(--text-muted);
  font-size: var(--text-lg);
}

.club-rating {
  display: flex;
  align-items: baseline;
  gap: var(--space-1);
  margin-top: var(--space-1);
}

.club-rating strong {
  font-size: var(--text-lg);
}

.shelf-panel {
  display: grid;
  gap: var(--space-3);
}

.status-control button {
  flex: 1 1 0;
}

.control-block {
  display: grid;
  gap: var(--space-2);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}

.control-block .field {
  margin-bottom: 0;
}

.control-block .btn {
  justify-self: start;
}

.progress-label {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
  font-size: var(--text-sm);
}

.control-block input[type="range"] {
  width: 100%;
  accent-color: var(--accent);
}

.panel-actions {
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}

.detail-intro .text-btn {
  display: inline;
  padding: 0;
  border: 0;
  background: none;
  font: inherit;
  color: var(--accent);
}

.description {
  white-space: pre-line;
  color: var(--text-muted);
  max-width: 68ch;
}

.description.clamped {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 6;
  line-clamp: 6;
  overflow: hidden;
}

.reader-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.reader-list li {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.reader-list a {
  font-weight: 600;
  color: inherit;
  text-decoration: none;
  margin-right: auto;
}

@media (min-width: 720px) {
  .detail-hero {
    grid-template-columns: 200px 1fr;
    gap: var(--space-6);
  }

  .detail-intro .display {
    font-size: var(--text-3xl);
  }
}

@media (min-width: 1024px) {
  .detail-hero {
    grid-template-columns: 240px minmax(0, 720px);
  }

  /* Wide enough to stack the shelf panel under the intro beside the cover. */
  .detail-side {
    display: grid;
    gap: var(--space-5);
    align-content: start;
    min-width: 0;
  }

  .detail-hero .shelf-panel {
    grid-column: auto;
  }

  .description {
    max-width: 720px;
  }
}
</style>
