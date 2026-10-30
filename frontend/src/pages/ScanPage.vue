<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import type { BookDetailOut, BookOut, EditionOut, IsbnHitOut, SearchHit, ShelfItemOut } from "../api/generated";
import AddBookSheet from "../components/AddBookSheet.vue";
import AddEditionForm from "../components/AddEditionForm.vue";
import BackLink from "../components/BackLink.vue";
import BarcodeScanner from "../components/BarcodeScanner.vue";
import BookCover from "../components/BookCover.vue";
import CoverPhotoButton from "../components/CoverPhotoButton.vue";
import { bookPath, STATUSES, statusLabel, statusShort, type Status } from "../constants";
import { parseIsbn } from "../isbn";
import { useToast } from "../stores/toast";

/**
 * `/scan` (#56): scan or type an ISBN, look it up once
 * (`GET /api/books/isbn/{isbn}`), then the hit card or the miss flow.
 *
 * - A hit: shelve it with this edition, or set this edition on the entry the
 *   reader already has; a photo when the edition has no cover.
 * - A miss: find the work by title and add the edition to it, or add a new
 *   book; either way the ISBN it was saved with is looked up (now a local hit)
 *   and shows as a hit card. The reader may change or clear the scanned ISBN
 *   in the form: a save without one opens the book.
 *
 * Nothing goes onto the shelf without a tap.
 */

type Phase = "scan" | "lookup" | "hit" | "miss" | "failed";

/** The lookup shows its seconds from here on (a German ISBN can take about 30 s). */
const WAIT_NOTE_AFTER_SEC = 2;
const FIND_LIMIT = 10;

const { t } = useI18n();
const router = useRouter();
const toast = useToast();
const findId = useId();

const phase = ref<Phase>("scan");
const isbn = ref("");
const hit = ref<IsbnHitOut | null>(null);
const failure = ref("");
const waitSec = ref(0);
const busy = ref(false);
/** The status this page shelved the book with. */
const shelvedAs = ref<Status | null>(null);
/** The reader's entry has this edition now (added or set here). */
const editionIsYours = ref(false);

const finding = ref(false);
const findInput = ref<HTMLInputElement | null>(null);
const findQuery = ref("");
const findBusy = ref(false);
const findError = ref("");
const works = ref<SearchHit[] | null>(null);
const chosenWork = ref<SearchHit | null>(null);
const addingBook = ref(false);

let lookupAbort: AbortController | null = null;
let clock: ReturnType<typeof setInterval> | null = null;
/** Bumped when the miss flow starts over; a search that comes back later is dropped. */
let findRun = 0;
/** The work a save in the miss flow went to: if its ISBN still misses, the book opens instead. */
let savedWork: number | null = null;

const coverBook = computed(() =>
  hit.value
    ? {
        id: hit.value.work_id,
        title: hit.value.title,
        authors: hit.value.authors,
        ol_work_key: hit.value.ol_work_key,
        cover_url: hit.value.cover_url,
        cover_thumb_url: hit.value.cover_thumb_url,
      }
    : null,
);
const hasCover = computed(() => Boolean(hit.value?.cover_url || hit.value?.cover_thumb_url));
const details = computed(() => {
  const current = hit.value;
  if (!current) return "";
  return [
    current.year ? String(current.year) : "",
    current.pages ? t("editions.pages", { n: current.pages }) : "",
    t("scan.isbn", { isbn: current.isbn }),
  ]
    .filter(Boolean)
    .join(" · ");
});

function byline(work: SearchHit): string {
  return [work.authors, work.year ? String(work.year) : ""].filter(Boolean).join(" · ");
}

function stopClock() {
  if (clock !== null) clearInterval(clock);
  clock = null;
  waitSec.value = 0;
}

function startClock() {
  stopClock();
  const started = Date.now();
  clock = setInterval(() => {
    waitSec.value = Math.floor((Date.now() - started) / 1000);
  }, 500);
}

function resetMiss() {
  findRun += 1;
  finding.value = false;
  findQuery.value = "";
  findBusy.value = false;
  findError.value = "";
  works.value = null;
  chosenWork.value = null;
  addingBook.value = false;
}

function abortLookup() {
  lookupAbort?.abort();
  lookupAbort = null;
  stopClock();
}

/** Looks `isbn` up; a 404 is the miss flow, 429 and 502 keep the ISBN for "Try again". */
async function lookup() {
  abortLookup();
  const controller = new AbortController();
  lookupAbort = controller;
  phase.value = "lookup";
  failure.value = "";
  startClock();
  try {
    const found = await api.lookupIsbn(isbn.value, controller.signal);
    if (controller.signal.aborted) return;
    hit.value = found;
    shelvedAs.value = null;
    editionIsYours.value = false;
    resetMiss();
    phase.value = "hit";
  } catch (err) {
    if (controller.signal.aborted) return;
    if (err instanceof ApiError && err.status === 404) {
      if (savedWork !== null) {
        // Never the miss card for a book just saved: it would invite a second, duplicate one.
        void router.push(bookPath(savedWork));
        return;
      }
      hit.value = null;
      resetMiss();
      phase.value = "miss";
    } else {
      failure.value = err instanceof ApiError ? err.message : t("errors.generic");
      phase.value = "failed";
    }
  } finally {
    if (lookupAbort === controller) {
      lookupAbort = null;
      stopClock();
    }
  }
}

function onIsbn(value: string) {
  savedWork = null;
  isbn.value = value;
  void lookup();
}

/** "Cancel" while looking up: back to the camera. */
function cancelLookup() {
  abortLookup();
  phase.value = "scan";
}

function scanAnother() {
  abortLookup();
  savedWork = null;
  resetMiss();
  hit.value = null;
  isbn.value = "";
  failure.value = "";
  phase.value = "scan";
}

/** The scanner's "Cancel": where the reader came from, else Discover. */
function leave() {
  const state = typeof window !== "undefined" ? window.history.state : null;
  if (state && typeof state.back === "string") router.back();
  else void router.push("/discover");
}

async function addTo(status: Status) {
  const current = hit.value;
  if (!current || busy.value) return;
  busy.value = true;
  try {
    let item: ShelfItemOut;
    try {
      item = await api.addToShelf({ work_id: current.work_id, edition_id: current.edition_id, status });
    } catch (err) {
      // Shelved meanwhile (another tab, the importer): that entry gets this edition and status.
      if (!(err instanceof ApiError && err.status === 409 && err.item)) throw err;
      item = await api.patchShelf(err.item.id, { edition_id: current.edition_id, status, position: 0 });
    }
    current.on_shelf = true;
    current.shelf_id = item.id;
    shelvedAs.value = item.status;
    editionIsYours.value = true;
    toast.show(t("scan.added", { status: statusLabel(item.status) }));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

/** The book is on the shelf already: this edition becomes the entry's (idempotent). */
async function useEdition() {
  const current = hit.value;
  if (!current?.shelf_id || busy.value) return;
  busy.value = true;
  try {
    await api.patchShelf(current.shelf_id, { edition_id: current.edition_id });
    editionIsYours.value = true;
    toast.show(t("editions.saved"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.saveFailed"));
  } finally {
    busy.value = false;
  }
}

function adoptCover(book: BookOut) {
  const current = hit.value;
  if (!current) return;
  current.cover_url = book.cover_url;
  current.cover_thumb_url = book.cover_thumb_url;
  current.cover_public_id = book.cover_public_id;
}

async function startFinding() {
  finding.value = true;
  await nextTick();
  findInput.value?.focus();
}

async function findWorks() {
  const q = findQuery.value.trim();
  if (!q || findBusy.value) return;
  const current = ++findRun;
  findBusy.value = true;
  findError.value = "";
  try {
    const page = await api.search({ q, limit: FIND_LIMIT });
    if (current !== findRun) return;
    works.value = page.items;
  } catch (err) {
    if (current !== findRun) return;
    works.value = null;
    findError.value = err instanceof ApiError ? err.message : t("errors.generic");
  } finally {
    if (current === findRun) findBusy.value = false;
  }
}

/**
 * A save in the miss flow. The ISBN it was saved with is in the catalog now, so its lookup is
 * local and shows the hit card; it need not be the scanned one, since the forms let the reader
 * change or clear it. A save without an ISBN opens the book.
 */
function showSaved(workId: number, savedIsbn: string | null) {
  addingBook.value = false;
  if (!savedIsbn) {
    void router.push(bookPath(workId));
    return;
  }
  savedWork = workId;
  isbn.value = savedIsbn;
  void lookup();
}

function editionAdded(edition: EditionOut) {
  toast.show(t("editions.added"));
  showSaved(edition.work_id, edition.isbn13);
}

/** The ISBN sent is an edition of the chosen work already ("Choose that edition"). */
function editionExists(_editionId: number, sent: string | null) {
  if (chosenWork.value) showSaved(chosenWork.value.id, sent ? parseIsbn(sent) : null);
}

function bookAdded(book: BookDetailOut, sent: string | null) {
  showSaved(book.id, sent ? parseIsbn(sent) : null);
}

onBeforeUnmount(() => {
  abortLookup();
  findRun += 1;
});
</script>

<template>
  <section class="scan-page">
    <BackLink :label="t('scan.back')" fallback="/discover" />
    <div class="page-head">
      <h1>{{ t("scan.title") }}</h1>
      <p v-if="phase === 'scan'" class="lede">{{ t("scan.lede") }}</p>
    </div>

    <BarcodeScanner v-if="phase === 'scan'" @isbn="onIsbn" @cancel="leave" />

    <div v-else-if="phase === 'lookup'" class="panel scan-panel" aria-live="polite" aria-busy="true">
      <p class="scan-wait">{{ t("scan.lookingUp", { isbn }) }}</p>
      <p v-if="waitSec >= WAIT_NOTE_AFTER_SEC" class="fine subtle">
        {{ t("scan.stillLooking", { n: waitSec }) }}
      </p>
      <div class="btn-row">
        <button class="btn btn-ghost btn-sm" type="button" @click="cancelLookup">
          {{ t("common.cancel") }}
        </button>
      </div>
    </div>

    <div v-else-if="phase === 'failed'" class="panel scan-panel" role="alert">
      <p class="fine subtle">{{ t("scan.isbn", { isbn }) }}</p>
      <p class="error">{{ failure }}</p>
      <div class="btn-row">
        <button class="btn btn-primary btn-sm" type="button" @click="lookup">
          {{ t("common.tryAgain") }}
        </button>
        <button class="btn btn-ghost btn-sm" type="button" @click="scanAnother">
          {{ t("scan.again") }}
        </button>
      </div>
    </div>

    <article
      v-else-if="phase === 'hit' && hit && coverBook"
      class="panel scan-panel hit-card"
      aria-labelledby="scan-hit-title"
    >
      <div class="hit-head">
        <BookCover :book="coverBook" size="lg" eager />
        <div class="hit-meta">
          <h2 id="scan-hit-title" class="hit-title">{{ hit.title }}</h2>
          <p v-if="hit.authors" class="muted">{{ hit.authors }}</p>
          <p class="fine subtle">{{ details }}</p>
        </div>
      </div>

      <div v-if="!hit.on_shelf" class="hit-section">
        <p class="caps subtle">{{ t("scan.addTo") }}</p>
        <div class="segmented" role="group" :aria-label="t('scan.addTo')">
          <button
            v-for="status in STATUSES"
            :key="status"
            type="button"
            :title="statusLabel(status)"
            :disabled="busy"
            @click="addTo(status)"
          >
            {{ statusShort(status) }}
          </button>
        </div>
      </div>
      <div v-else class="hit-section">
        <p class="on-shelf">
          {{ shelvedAs ? t("scan.added", { status: statusLabel(shelvedAs) }) : t("scan.onShelf") }}
        </p>
        <p v-if="editionIsYours" class="fine subtle">{{ t("scan.editionIsYours") }}</p>
        <div v-else class="btn-row">
          <button class="btn btn-primary btn-sm" type="button" :disabled="busy" @click="useEdition">
            {{ t("scan.useEdition") }}
          </button>
        </div>
      </div>

      <div v-if="!hasCover" class="hit-section">
        <p class="fine muted">{{ t("scan.noCover") }}</p>
        <div>
          <CoverPhotoButton
            :work-id="hit.work_id"
            :edition-id="hit.edition_id"
            :has-cover="false"
            :label="t('scan.photo')"
            look="ghost"
            @covered="adoptCover"
          />
        </div>
      </div>

      <div class="btn-row">
        <RouterLink class="btn btn-ghost btn-sm" :to="bookPath(hit.work_id)">
          {{ t("scan.openBook") }}
        </RouterLink>
        <button class="btn btn-ghost btn-sm" type="button" @click="scanAnother">
          {{ t("scan.again") }}
        </button>
      </div>
    </article>

    <section v-else-if="phase === 'miss'" class="panel scan-panel" aria-labelledby="scan-miss-title">
      <h2 id="scan-miss-title" class="miss-title">{{ t("scan.missTitle") }}</h2>
      <p class="fine subtle">{{ t("scan.isbn", { isbn }) }}</p>
      <p class="muted">{{ t("scan.missBody") }}</p>

      <div v-if="chosenWork" class="hit-section chosen">
        <p class="fine">
          <strong>{{ t("scan.editionOf", { isbn, title: chosenWork.title }) }}</strong>
        </p>
        <AddEditionForm
          :work-id="chosenWork.id"
          :initial-isbn="isbn"
          @added="editionAdded"
          @existing="editionExists"
          @cancel="chosenWork = null"
        />
      </div>
      <div v-else-if="finding" class="hit-section">
        <form class="field find" role="search" @submit.prevent="findWorks">
          <label class="find-label" :for="findId">{{ t("scan.findLabel") }}</label>
          <div class="find-row">
            <input
              :id="findId"
              ref="findInput"
              v-model="findQuery"
              type="search"
              autocomplete="off"
              enterkeyhint="search"
              maxlength="200"
            />
            <button class="btn btn-primary" type="submit" :disabled="findBusy">{{ t("scan.find") }}</button>
          </div>
        </form>
        <p v-if="findError" class="error">{{ findError }}</p>
        <ul v-if="works?.length" class="work-list">
          <li v-for="work in works" :key="work.id">
            <button type="button" class="work-row" @click="chosenWork = work">
              <BookCover :book="work" size="xs" />
              <span class="work-meta">
                <strong>{{ work.title }}</strong>
                <span v-if="byline(work)" class="fine muted">{{ byline(work) }}</span>
              </span>
            </button>
          </li>
        </ul>
        <p v-else-if="works" class="fine muted">{{ t("scan.noWorks") }}</p>
      </div>

      <div class="btn-row">
        <button
          v-if="!finding && !chosenWork"
          class="btn btn-primary btn-sm"
          type="button"
          @click="startFinding"
        >
          {{ t("scan.findByTitle") }}
        </button>
        <button class="btn btn-ghost btn-sm" type="button" @click="addingBook = true">
          {{ t("scan.addNew") }}
        </button>
        <button class="btn btn-ghost btn-sm" type="button" @click="scanAnother">
          {{ t("scan.again") }}
        </button>
      </div>

      <AddBookSheet
        v-if="addingBook"
        :initial-isbn="isbn"
        stay
        @added="bookAdded"
        @close="addingBook = false"
      />
    </section>
  </section>
</template>

<style scoped>
.scan-page {
  max-width: 640px;
}

.scan-panel {
  display: grid;
  gap: var(--space-3);
}

.scan-panel p {
  margin: 0;
}

.scan-wait {
  font-weight: 600;
  overflow-wrap: anywhere;
}

.hit-head {
  display: flex;
  gap: var(--space-4);
  align-items: flex-start;
}

.hit-meta {
  display: grid;
  gap: var(--space-1);
  min-width: 0;
}

.hit-title,
.miss-title {
  margin: 0;
  font-family: var(--serif);
  font-size: var(--text-lg);
  overflow-wrap: anywhere;
}

.hit-section {
  display: grid;
  gap: var(--space-2);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}

.on-shelf {
  font-weight: 600;
}

/* The section's own line already separates the form. */
.chosen :deep(.add-edition) {
  margin-top: 0;
  padding-top: 0;
  border-top: 0;
}

.find {
  margin: 0;
}

.find-label {
  font-size: var(--text-sm);
  font-weight: 600;
}

.find-row {
  display: flex;
  gap: var(--space-2);
}

.find-row input {
  flex: 1;
  min-width: 0;
}

.work-list {
  display: grid;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}

.work-row {
  display: flex;
  gap: var(--space-3);
  align-items: center;
  width: 100%;
  padding: var(--space-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.work-row:hover {
  background: var(--surface-2);
}

.work-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
  overflow-wrap: anywhere;
}
</style>
