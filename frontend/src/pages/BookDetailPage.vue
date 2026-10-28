<script setup lang="ts">
import { vAutosize } from "../autosize";
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import BookDiary from "../components/BookDiary.vue";
import QuoteList from "../components/QuoteList.vue";
import ClubBookActions from "../components/ClubBookActions.vue";
import ReportCoverSheet from "../components/ReportCoverSheet.vue";
import EditionPicker from "../components/EditionPicker.vue";
import CoverPhotoButton from "../components/CoverPhotoButton.vue";
import MergeRequestSheet from "../components/MergeRequestSheet.vue";
import EditRequestSheet from "../components/EditRequestSheet.vue";
import { clubsFor } from "../clubActions";
import { worthRetrying } from "../retry";
import {
  bookPath,
  openLibraryUrl,
  parseBookParam,
  STATUSES,
  statusLabel,
  statusShort,
  starLabel,
  type Status,
} from "../constants";
import { tp } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { BookOut } from "../api/generated";
import type { BookDetail, ClubRef, ShelfItem, Visibility } from "../types";

const { t } = useI18n();

const route = useRoute();
const router = useRouter();
const session = useSession();
const toast = useToast();

const book = ref<BookDetail | null>(null);
const error = ref("");
/** The failed load may work a second time (network, 5xx); a 404 or a bad id never does. */
const retryable = ref(false);
const loaded = ref(false);
const loadWaitSec = ref(0);
const busy = ref(false);
const refreshing = ref(false);
const expanded = ref(false);
const reportingCover = ref(false);
const requestingMerge = ref(false);
const requestingEdit = ref(false);

const takeDraft = ref("");
const reasonDraft = ref("");
const progressDraft = ref(0);
const diary = ref<InstanceType<typeof BookDiary> | null>(null);
const clubActions = ref<InstanceType<typeof ClubBookActions> | null>(null);

const param = computed(() => String(route.params.id || ""));
const longDescription = computed(() => (book.value?.description.length ?? 0) > 420);
const catalogUrl = computed(() => openLibraryUrl(book.value?.ol_work_key));
const canPick = computed(() => clubsFor("pick", session.user?.clubs).length > 0);
const canNominate = computed(() => clubsFor("nominate", session.user?.clubs).length > 0);
/** With one club to pick in, and the book already its pick, there is nothing to set. */
const pickDone = computed(() => {
  const clubs = clubsFor("pick", session.user?.clubs);
  const current = book.value?.club_picks ?? [];
  return clubs.length > 0 && clubs.every((club) => current.some((row) => row.slug === club.slug));
});
const pickNames = computed(() => (book.value?.club_picks ?? []).map((club) => club.name).join(", "));

function syncDrafts(detail: BookDetail) {
  takeDraft.value = detail.take;
  reasonDraft.value = detail.dnf_reason;
  progressDraft.value = detail.progress ?? 0;
}

/** A photo became the cover: show it without reloading the page. */
function adoptCover(updated: BookOut) {
  const detail = book.value;
  if (!detail) return;
  detail.cover_url = updated.cover_url;
  detail.cover_thumb_url = updated.cover_thumb_url;
  detail.cover_public_id = updated.cover_public_id;
}

/** Copy the viewer's shelf entry onto the page after a write. */
function adoptEntry(detail: BookDetail, item: ShelfItem) {
  detail.shelf_id = item.id;
  detail.on_shelf = item.status;
  detail.edition_id = item.edition?.id ?? null;
  detail.rating = item.rating;
  detail.rating_visibility = item.rating_visibility;
  detail.take = item.take;
  detail.dnf_reason = item.dnf_reason;
  detail.progress = item.progress;
  syncDrafts(detail);
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
  const target = parseBookParam(param.value);
  // The legacy key or merged id we just resolved now reads /book/<id>: same book, no second fetch.
  if (target.kind === "id" && book.value?.id === target.id && loaded.value && !error.value) return;
  loaded.value = false;
  error.value = "";
  retryable.value = false;
  expanded.value = false;
  loadAbort?.abort();
  if (target.kind === "invalid") {
    book.value = null;
    error.value = t("book.notFound");
    loaded.value = true;
    return;
  }
  const controller = new AbortController();
  loadAbort = controller;
  startLoadClock();
  try {
    const detail = await api.book(target.kind === "id" ? target.id : target.key, {
      signal: controller.signal,
    });
    if (controller.signal.aborted) return;
    book.value = detail;
    syncDrafts(detail);
    // Old links (/book/OL…W, BC…, ISBN…, GB…) and merged works land on the work's
    // own id; the #entry- hash of a diary link comes along.
    if (target.kind === "legacy" || target.id !== detail.id) {
      void router.replace({ path: bookPath(detail), query: route.query, hash: route.hash });
    }
  } catch (err) {
    if (
      (err instanceof DOMException && err.name === "AbortError") ||
      (err instanceof Error && err.name === "AbortError")
    ) {
      return;
    }
    book.value = null;
    error.value = err instanceof ApiError ? err.message : t("book.loadFailed");
    retryable.value = worthRetrying(err);
  } finally {
    if (!controller.signal.aborted) {
      loaded.value = true;
      stopLoadClock();
    }
  }
}

async function setStatus(status: Status) {
  const detail = book.value;
  if (!detail || busy.value) return;
  busy.value = true;
  try {
    const item = detail.shelf_id
      ? await api.patchShelf(detail.shelf_id, { status, position: 0 })
      : await api.addToShelf({ work_id: detail.id, status });
    adoptEntry(detail, item);
    toast.show(t("book.movedTo", { status: statusLabel(status) }));
  } catch (err) {
    // Another tab or the importer may have shelved it since this page loaded.
    if (err instanceof ApiError && err.status === 409 && err.item) {
      adoptEntry(detail, err.item);
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
    adoptEntry(detail, await api.patchShelf(detail.shelf_id, body));
    toast.show(message, action);
    if ("rating" in body || "rating_visibility" in body) void refreshClubRating();
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

/** The average moved with my stars; pull it without disturbing the page. */
async function refreshClubRating() {
  const detail = book.value;
  if (!detail) return;
  try {
    const fresh = await api.book(detail.id);
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

async function setRatingVisibility(event: Event) {
  const box = event.target as HTMLInputElement;
  const visibility: Visibility = box.checked ? "only_me" : "audience";
  await patch({ rating_visibility: visibility }, t("book.ratingVisibilitySaved"));
  // The box shows what the server stored: after a failed save the click is undone.
  box.checked = book.value?.rating_visibility === "only_me";
}

async function remove() {
  const detail = book.value;
  if (!detail?.shelf_id) return;
  const previous = {
    work_id: detail.id,
    edition_id: detail.edition_id,
    status: detail.on_shelf as Status,
    rating: detail.rating,
    rating_visibility: detail.rating_visibility,
    take: detail.take,
    dnf_reason: detail.dnf_reason,
    progress: detail.progress,
  };
  busy.value = true;
  try {
    await api.removeFromShelf(detail.shelf_id);
    detail.shelf_id = null;
    detail.on_shelf = null;
    detail.edition_id = null;
    detail.rating = null;
    detail.rating_visibility = null;
    detail.take = "";
    detail.dnf_reason = "";
    detail.progress = null;
    syncDrafts(detail);
    toast.show(t("book.removed"), {
      label: t("common.undo"),
      run: async () => {
        adoptEntry(detail, await api.addToShelf(previous));
      },
    });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("shelf.removeFailed"));
  } finally {
    busy.value = false;
  }
}

function startClubAction(action: "pick" | "nominate") {
  const detail = book.value;
  if (!detail) return;
  clubActions.value?.start(action, { id: detail.id, title: detail.title, currentIn: detail.club_picks });
}

function onPicked(club: ClubRef) {
  const detail = book.value;
  if (detail && !detail.club_picks.some((row) => row.slug === club.slug)) {
    detail.club_picks = [...detail.club_picks, club];
  }
}

async function refreshDetails() {
  const detail = book.value;
  if (!detail || refreshing.value) return;
  refreshing.value = true;
  try {
    const fresh = await api.refreshBook(detail.id);
    book.value = fresh;
    syncDrafts(fresh);
    toast.show(t("book.detailsUpdated"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.refreshFailed"));
  } finally {
    refreshing.value = false;
  }
}

async function toggleFavorite() {
  const detail = book.value;
  if (!detail || busy.value) return;
  busy.value = true;
  try {
    if (detail.favorite_position) {
      await api.removeFavorite(detail.id);
      detail.favorite_position = null;
      toast.show(t("favorites.removed"));
    } else {
      const result = await api.addFavorite(detail.id);
      const mine = result.items.find((row) => row.book.id === detail.id);
      detail.favorite_position = mine?.position ?? null;
      toast.show(t("favorites.added"));
    }
  } catch (err) {
    if (err instanceof ApiError && err.status === 409) {
      toast.show(t("favorites.full"));
    } else {
      toast.show(err instanceof ApiError ? err.message : t("favorites.saveFailed"));
    }
  } finally {
    busy.value = false;
  }
}

onMounted(load);
onUnmounted(() => {
  loadAbort?.abort();
  stopLoadClock();
});
watch(param, load);
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
        <button v-if="retryable" class="btn btn-ghost" type="button" @click="load">
          {{ t("common.tryAgain") }}
        </button>
        <RouterLink class="btn btn-ghost" to="/discover">{{ t("book.backToDiscover") }}</RouterLink>
      </div>
    </div>

    <template v-else-if="book">
      <div class="detail-hero">
        <div class="hero-cover">
          <BookCover :book="book" eager />
          <button
            v-if="book.cover_public_id"
            class="text-btn finer report-cover"
            type="button"
            @click="reportingCover = true"
          >
            {{ t("cover.report") }}
          </button>
          <CoverPhotoButton
            :work-id="book.id"
            :edition-id="book.edition_id"
            :has-cover="Boolean(book.cover_url || book.cover_thumb_url)"
            @covered="adoptCover"
          />
        </div>
        <div class="detail-side">
          <div class="detail-intro">
            <p v-if="book.club_picks.length" class="kicker">{{ t("book.currentPickIn", { clubs: pickNames }) }}</p>
            <h1 class="display">{{ book.title }}</h1>
            <p v-if="book.subtitle" class="detail-subtitle">{{ book.subtitle }}</p>
            <p v-if="book.authors" class="detail-authors">{{ book.authors }}</p>
            <p v-if="book.rating_count" class="club-rating">
              <span class="stars" aria-hidden="true">★</span>
              <strong class="nums">{{ book.club_rating?.toFixed(1) }}</strong>
              <span class="fine subtle">
                {{ tp("book.clubRating", book.rating_count) }}
              </span>
            </p>
            <p v-if="book.year" class="fine subtle nums">{{ t("book.firstPublished", { year: book.year }) }}</p>
            <p v-if="book.custom" class="fine subtle">
              {{ t("book.addedByReader") }}
            </p>
            <p v-else-if="catalogUrl || book.can_refresh" class="fine">
              <a
                v-if="catalogUrl"
                :href="catalogUrl"
                target="_blank"
                rel="noreferrer"
              >
                {{ t("book.viewOpenLibrary") }}
              </a>
              <span v-if="catalogUrl && book.can_refresh" class="subtle"> · </span>
              <button
                v-if="book.can_refresh"
                class="text-btn"
                type="button"
                :disabled="refreshing"
                @click="refreshDetails"
              >
                {{ refreshing ? t("book.refreshing") : t("book.refreshDetails") }}
              </button>
            </p>
            <p class="fine catalog-requests">
              <button class="text-btn" type="button" @click="requestingEdit = true">{{ t("editRequest.openBook") }}</button>
              <span class="subtle" aria-hidden="true"> · </span>
              <button class="text-btn" type="button" @click="requestingMerge = true">{{ t("merge.open") }}</button>
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
              <label class="check-row">
                <input
                  type="checkbox"
                  :checked="book.rating_visibility === 'only_me'"
                  :disabled="busy"
                  @change="setRatingVisibility"
                />
                <span class="fine">{{ t("book.ratingOnlyMe") }}</span>
              </label>
              <label class="field">
                <span class="field-head">{{ t("book.oneLineTake") }}<span class="finer subtle nums">{{ takeDraft.length }}/140</span></span>
                <textarea
                  v-model="takeDraft"
                  v-autosize
                  class="short-note"
                  rows="2"
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
                <span class="field-head">{{ t("book.dnfWhy") }}<span class="finer subtle nums">{{ reasonDraft.length }}/200</span></span>
                <textarea
                  v-model="reasonDraft"
                  v-autosize
                  class="short-note"
                  rows="2"
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

            <EditionPicker
              v-if="book.shelf_id"
              :work-id="book.id"
              :edition-id="book.edition_id"
              :disabled="busy"
              @choose="(editionId) => patch({ edition_id: editionId }, t('editions.saved'))"
            />

            <div class="btn-row panel-actions">
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                :aria-pressed="Boolean(book.favorite_position)"
                :disabled="busy"
                @click="toggleFavorite"
              >
                {{
                  book.favorite_position
                    ? t("favorites.markedAt", { n: book.favorite_position })
                    : t("favorites.mark")
                }}
              </button>
              <button
                v-if="canPick"
                class="btn btn-ghost btn-sm"
                type="button"
                :disabled="pickDone"
                @click="startClubAction('pick')"
              >
                {{ pickDone ? t("book.alreadyPick") : t("book.setAsPick") }}
              </button>
              <button
                v-if="canNominate"
                class="btn btn-ghost btn-sm"
                type="button"
                @click="startClubAction('nominate')"
              >
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
        <BookDiary ref="diary" :work-id="book.id" />
      </section>

      <section class="section">
        <QuoteList :work-id="book.id" :empty-text="t('quotes.emptyBook')" />
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
            <RouterLink :to="`/u/${reader.username}`">{{ reader.username }}</RouterLink>
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

    <ClubBookActions ref="clubActions" @picked="onPicked" />
    <MergeRequestSheet
      v-if="requestingMerge && book"
      :work-id="book.id"
      :title="book.title"
      @close="requestingMerge = false"
    />
    <EditRequestSheet v-if="requestingEdit && book" :book="book" @close="requestingEdit = false" />
    <ReportCoverSheet
      v-if="reportingCover && book?.cover_public_id"
      :public-id="book.cover_public_id"
      :title="book.title"
      @close="reportingCover = false"
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

.report-cover,
.hero-cover :deep(.cover-photo) {
  display: block;
  margin-top: var(--space-1);
  color: var(--text-subtle);
}

.check-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--tap);
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

.detail-subtitle {
  margin-top: calc(-1 * var(--space-1));
  font-family: var(--serif);
  font-size: var(--text-lg);
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

.field-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.field-head .finer {
  font-weight: 400;
}

.field textarea.short-note {
  min-height: 4.25rem;
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
