<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import BookDiary from "../components/BookDiary.vue";
import MeetingSheet from "../components/MeetingSheet.vue";
import {
  catalogUrl as externalCatalogUrl,
  isbnFromWorkKey,
  isClubWorkId,
  isOpenLibraryWorkId,
  openLibraryUrl,
  STATUSES,
  statusLabel,
  statusShort,
  starLabel,
  type Status,
} from "../constants";
import { tp } from "../i18n";
import { useToast } from "../stores/toast";
import type { BookDetail } from "../types";

const { t } = useI18n();

const route = useRoute();
const toast = useToast();

const book = ref<BookDetail | null>(null);
const error = ref("");
const loaded = ref(false);
const loadWaitSec = ref(0);
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
const customBook = computed(
  () => book.value?.custom === true || isClubWorkId(workId.value),
);
const catalogUrl = computed(() =>
  book.value ? externalCatalogUrl(book.value.ol_work_key) : null,
);
const catalogLabel = computed(() => {
  if (!book.value) return "";
  if (openLibraryUrl(book.value.ol_work_key)) return t("book.viewOpenLibrary");
  return t("book.viewGoogle");
});
const catalogIsbn = computed(() =>
  book.value ? isbnFromWorkKey(book.value.ol_work_key) : null,
);
const fromOpenLibrary = computed(() => isOpenLibraryWorkId(workId.value));

function syncDrafts(detail: BookDetail) {
  takeDraft.value = detail.take;
  reasonDraft.value = detail.dnf_reason;
  progressDraft.value = detail.progress ?? 0;
}

let loadClock: ReturnType<typeof setInterval> | null = null;
let loadAbort: AbortController | null = null;

function stopLoadClock() {
  if (loadClock !== null) clearInterval(loadClock);
  loadClock = null;
  loadWaitSec.value = 0;
}

function startLoadClock() {
  const started = Date.now();
  stopLoadClock();
  loadClock = window.setInterval(() => {
    loadWaitSec.value = Math.floor((Date.now() - started) / 1000);
  }, 500);
}

async function load() {
  loaded.value = false;
  error.value = "";
  expanded.value = false;
  loadAbort?.abort();
  const controller = new AbortController();
  loadAbort = controller;
  startLoadClock();
  try {
    const detail = await api.book(workId.value, { signal: controller.signal });
    if (controller.signal.aborted) return;
    book.value = detail;
    syncDrafts(detail);
  } catch (err) {
    if (
      (err instanceof DOMException && err.name === "AbortError") ||
      (err instanceof Error && err.name === "AbortError")
    ) {
      return;
    }
    book.value = null;
    error.value = err instanceof ApiError ? err.message : t("book.loadFailed");
  } finally {
    if (!controller.signal.aborted) {
      loaded.value = true;
      stopLoadClock();
    }
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
        cover_url: detail.cover_url,
        year: detail.year,
        status,
      });
      detail.shelf_id = created.id;
    }
    detail.on_shelf = status;
    toast.show(t("book.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    // Another tab or the importer may have shelved it since this page loaded.
    if (err instanceof ApiError && err.status === 409 && err.item) {
      detail.shelf_id = err.item.id;
      detail.on_shelf = err.item.status;
      busy.value = false;
      await setStatus(status);
      return;
    }
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
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
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
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
    void patch({ rating: null }, t("book.ratingCleared"));
    return;
  }
  void patch({ rating: value }, t("book.starsOf", { n: value }), {
    label: t("book.writeAFew"),
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
    toast.show(t("book.removed"), {
      label: t("common.undo"),
      run: async () => {
        const created = await api.addToShelf({
          ol_work_key: detail.ol_work_key,
          title: detail.title,
          authors: detail.authors,
          cover_id: detail.cover_id,
          cover_url: detail.cover_url,
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
    toast.show(err instanceof ApiError ? err.message : t("shelf.removeFailed"));
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
      cover_url: detail.cover_url,
      year: detail.year,
      meeting_at: meetingAt,
    });
    detail.club_pick = true;
    toast.show(t("book.setPick"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.setPickFailed"));
  }
}

async function refreshDetails() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    const detail = await api.refreshBook(workId.value);
    book.value = detail;
    syncDrafts(detail);
    toast.show(t("book.detailsUpdated"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.refreshFailed"));
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
      cover_url: detail.cover_url,
      year: detail.year,
    });
    toast.show(t("book.nominated"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.nominateFailed"));
  }
}

onMounted(load);
onUnmounted(() => {
  loadAbort?.abort();
  stopLoadClock();
});
watch(workId, load);
</script>

<template>
  <section>
    <p class="fine back-link">
      <RouterLink to="/discover">{{ t("book.backDiscover") }}</RouterLink>
    </p>

    <template v-if="!loaded">
      <div class="detail-hero" aria-hidden="true">
        <span class="skeleton hero-cover-skeleton" />
        <div class="detail-intro">
          <span class="skeleton skeleton-line" />
          <span class="skeleton skeleton-line short" />
        </div>
      </div>
      <p v-if="loadWaitSec >= 2" class="fine subtle" aria-live="polite">
        {{ t("book.stillLoading", { n: loadWaitSec }) }}
      </p>
    </template>

    <div v-else-if="error" class="empty">
      <h3>{{ t("book.loadFailedTitle") }}</h3>
      <p>{{ error }}</p>
      <div class="btn-row">
        <button class="btn btn-ghost" type="button" @click="load">{{ t("common.tryAgain") }}</button>
        <RouterLink class="btn btn-ghost" to="/discover">{{ t("book.backToDiscover") }}</RouterLink>
      </div>
    </div>

    <template v-else-if="book">
      <div class="detail-hero">
        <div class="hero-cover">
          <BookCover
            :title="book.title"
            :cover-id="book.cover_id"
            :isbn="catalogIsbn"
            :work-key="book.ol_work_key"
            :image-url="book.cover_url"
            eager
          />
        </div>
        <div class="detail-side">
          <div class="detail-intro">
            <p v-if="book.club_pick" class="kicker">{{ t("book.currentPick") }}</p>
            <h1 class="display">{{ book.title }}</h1>
            <p v-if="book.authors" class="detail-authors">{{ book.authors }}</p>
            <p v-if="book.rating_count" class="club-rating">
              <span class="stars" aria-hidden="true">★</span>
              <strong class="nums">{{ book.club_rating?.toFixed(1) }}</strong>
              <span class="fine subtle">
                {{ tp("book.clubRating", book.rating_count) }}
              </span>
            </p>
            <p v-if="book.year" class="fine subtle nums">{{ t("book.firstPublished", { year: book.year }) }}</p>
            <p v-if="customBook" class="fine subtle">
              {{ t("book.addedByClub") }}
            </p>
            <p v-else class="fine">
              <a
                v-if="catalogUrl"
                :href="catalogUrl"
                target="_blank"
                rel="noreferrer"
              >
                {{ catalogLabel }}
              </a>
              <span v-if="catalogUrl" class="subtle"> · </span>
              <button
                class="text-btn"
                type="button"
                :disabled="refreshing"
                @click="refreshDetails"
              >
                {{ refreshing ? t("book.refreshing") : t("book.refreshDetails") }}
              </button>
            </p>
            <p v-if="!book.cover_id && !book.cover_url && !customBook && fromOpenLibrary" class="fine">
              {{ t("book.noCover") }}
              <button
                class="text-btn"
                type="button"
                :disabled="refreshing"
                @click="refreshDetails"
              >
                {{ t("book.refreshFromOl") }}
              </button>
            </p>
          </div>

          <div class="panel shelf-panel">
            <p class="caps subtle">{{ t("book.yourShelf") }}</p>
            <div class="segmented status-control" role="group" :aria-label="t('book.shelfStatus')">
              <button
                v-for="status in STATUSES"
                :key="status"
                type="button"
                :aria-pressed="book.on_shelf === status"
                :disabled="busy"
                @click="setStatus(status)"
              >
                {{ statusShort(status) }}
              </button>
            </div>

            <p v-if="!book.on_shelf" class="fine subtle">
              {{ t("book.notOnShelf") }}
            </p>

            <div v-if="book.on_shelf === 'currently_reading'" class="control-block">
              <label class="progress-label" for="progress">
                <span>{{ t("book.progress") }}</span>
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
                @change="patch({ progress: progressDraft }, t('book.progressSaved', { n: progressDraft }))"
              />
            </div>

            <div v-if="book.on_shelf === 'finished'" class="control-block">
              <span class="fine">{{ t("book.yourRating") }}</span>
              <div class="star-row">
                <button
                  v-for="value in 5"
                  :key="value"
                  class="star-btn"
                  type="button"
                  :aria-label="t('book.starsOf', { n: value })"
                  :aria-pressed="book.rating === value"
                  :disabled="busy"
                  @click="rate(value)"
                >
                  {{ book.rating && book.rating >= value ? "★" : "☆" }}
                </button>
              </div>
              <label class="field">
                <span>{{ t("book.oneLineTake") }}</span>
                <input
                  v-model="takeDraft"
                  type="text"
                  maxlength="140"
                  :placeholder="t('book.takePlaceholder')"
                />
              </label>
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busy || takeDraft === book.take"
                @click="patch({ take: takeDraft }, t('book.takeSaved'))"
              >
                {{ t("book.saveTake") }}
              </button>
            </div>

            <div v-if="book.on_shelf === 'did_not_finish'" class="control-block">
              <label class="field">
                <span>{{ t("book.dnfWhy") }}</span>
                <input
                  v-model="reasonDraft"
                  type="text"
                  maxlength="200"
                  :placeholder="t('common.optional')"
                />
              </label>
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="busy || reasonDraft === book.dnf_reason"
                @click="patch({ dnf_reason: reasonDraft }, t('book.reasonSaved'))"
              >
                {{ t("book.saveReason") }}
              </button>
            </div>

            <div class="btn-row panel-actions">
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="book.club_pick"
                @click="settingPick = true"
              >
                {{ book.club_pick ? t("book.alreadyPick") : t("book.setAsPick") }}
              </button>
              <button class="btn btn-ghost btn-sm" type="button" @click="nominate">
                {{ t("book.nominate") }}
              </button>
              <button
                v-if="book.on_shelf"
                class="btn btn-danger btn-sm"
                type="button"
                :disabled="busy"
                @click="remove"
              >
                {{ t("common.remove") }}
              </button>
            </div>
          </div>
        </div>
      </div>

      <section v-if="book.description" class="section">
        <div class="section-head">
          <h2>{{ t("book.about") }}</h2>
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
          {{ expanded ? t("book.showLess") : t("book.readMore") }}
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
            cover_url: book.cover_url,
            year: book.year,
          }"
        />
      </section>

      <section v-if="book.subjects.length" class="section">
        <div class="section-head">
          <h2>{{ t("book.subjects") }}</h2>
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
          <h2>{{ t("book.otherReaders") }}</h2>
        </div>
        <ul v-if="book.readers.length" class="reader-list">
          <li v-for="reader in book.readers" :key="reader.username">
            <Avatar :username="reader.username" :src="reader.avatar_url" size="sm" />
            <RouterLink :to="`/club/${reader.username}`">{{ reader.username }}</RouterLink>
            <span class="badge" :class="reader.status">
              {{ statusShort(reader.status) }}
            </span>
            <span v-if="reader.progress != null" class="fine subtle nums">
              {{ reader.progress }}%
            </span>
            <span v-if="reader.rating" class="fine stars">
              {{ starLabel(reader.rating) }}
            </span>
          </li>
        </ul>
        <p v-else class="fine subtle">{{ t("book.nobodyElse") }}</p>
      </section>
    </template>

    <MeetingSheet
      v-if="settingPick && book"
      :title="t('book.setAsPick')"
      :book-title="book.title"
      :timezone="clubTimezone"
      :blurb="t('book.pickBlurb')"
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
