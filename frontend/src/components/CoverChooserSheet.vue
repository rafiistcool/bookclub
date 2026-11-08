<script lang="ts">
import { api, ApiError } from "../api/client";
import { i18n } from "../i18n";
import { useCoverScan } from "../stores/coverScan";
import { useToast } from "../stores/toast";
import type { ShelfItem } from "../types";

const MAX_BYTES = 50 * 1024 * 1024;

/**
 * What an Undo puts back: a candidate's `key` (`media:…` or `ol:…`), or
 * `null` for the standard cover. A change without a known previous cover
 * (`CoverRestore | null` = null) gets no Undo.
 */
export type CoverRestore = { key: string | null };

/**
 * Straighten the reader's own photo in the cover scanner and make what they
 * kept their cover of this entry. Resolves to the entry, or null when they
 * cancelled the scan or the upload failed (a toast says why). The page calls
 * it after the chooser closed, so the answer reaches the page.
 */
export async function sendPersonalCover(entryId: number, chosen: File): Promise<ShelfItem | null> {
  const t = i18n.global.t;
  const toast = useToast();
  const file = await useCoverScan().request(chosen);
  if (!file) return null;
  if (file.size > MAX_BYTES) {
    toast.show(t("errors.pictureTooLarge"));
    return null;
  }
  try {
    return await api.uploadPersonalCover(entryId, file);
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("cover.photoFailed"));
    return null;
  }
}

/** Put a previous cover back: choose its key again, or go back to the standard cover. */
export function restoreCover(entryId: number, restore: CoverRestore): Promise<ShelfItem> {
  return restore.key ? api.chooseCover(entryId, restore.key) : api.clearPersonalCover(entryId);
}

/**
 * "Cover changed", with an Undo when the previous cover is known. The Undo
 * hands the entry it restored to `adopt`; the chooser is long gone by then.
 */
export function announceCoverChange(entryId: number, restore: CoverRestore | null, adopt: (item: ShelfItem) => void) {
  const t = i18n.global.t;
  const toast = useToast();
  if (!restore) {
    toast.show(t("coverChoice.changed"));
    return;
  }
  toast.show(t("coverChoice.changed"), {
    label: t("common.undo"),
    run: async () => {
      try {
        adopt(await restoreCover(entryId, restore));
      } catch (err) {
        toast.show(err instanceof ApiError ? err.message : t("coverChoice.undoFailed"));
      }
    },
  });
}
/**
 * A publisher as the tile shows it: without the legal form at its end
 * ("Diogenes Verlag AG" -> "Diogenes Verlag", "Penguin Random House LLC"
 * -> "Penguin Random House"), which tells no edition apart and cost the
 * name its last word. The full name stays in the tile's tooltip.
 */
export function shortPublisher(name: string): string {
  const legal = /(?:[\s,]+(?:G?mbH(?:\s*[&+]\s*Co\.?)?|KG(?:aA)?|AG|e\.\s?K\.|OHG|Ltd\.?|LLC|Inc\.?|S\.A\.|S\.p\.A\.|B\.V\.|plc))+\.?$/i;
  return name.trim().replace(legal, "").replace(/[\s,&]+$/, "").trim() || name.trim();
}
</script>

<script setup lang="ts">
import { ArrowUturnLeftIcon, CameraIcon, CheckIcon } from "@heroicons/vue/24/outline";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { CoverCandidateOut } from "../types";
import { COVER_RETRY_MS, coverSources } from "../covers";
import { isRealLanguage, languageName } from "../languages";
import BottomSheet from "./BottomSheet.vue";
import CoverPhotoButton from "./CoverPhotoButton.vue";

/**
 * "Cover wählen": the reader's own cover of a book on their shelf. It shows
 * on their shelf (to everyone who may look at it), on Home and on the book
 * page for them; everyone else keeps the standard cover. Instance admins set
 * the cover for everyone elsewhere (the "…" sheet's admin rows).
 *
 * A grid of the candidates' thumbnails, all from our own `/media` (the server
 * fetches Open Library's pictures on the first request, so each `<img>` loads
 * lazily). A picture that fails is asked for once more after a pause, like
 * `BookCover` (a busy or slow Open Library answers 429 or 502 for a moment),
 * then in the large size; a tile whose pictures all fail goes away. A tap applies the choice
 * at once: the ring moves to the tile, then the sheet closes and the page
 * shows "Cover changed" with an Undo (`changed`).
 *
 * "Own photo" is a real file input (`CoverPhotoButton` with `pickOnly`, the
 * tap opens the picker as iOS requires). The chosen file is handed on
 * (`picked`) for the page to straighten in the cover scanner and upload with
 * `sendPersonalCover`, after this sheet closed: the scanner is no sheet on a
 * sheet, and the answer is not lost with an unmounted sheet.
 */
const props = defineProps<{
  entryId: number;
  workId: number;
  /** The book's title as the page shows it. */
  title: string;
  /** The entry shows the reader's own cover now. */
  personalCover: boolean;
  /** The picture the entry shows now, for an Undo when no candidate is marked `chosen`. */
  coverPublicId: string | null;
}>();
const emit = defineEmits<{
  close: [];
  changed: [item: ShelfItem, restore: CoverRestore | null];
  picked: [file: File, restore: CoverRestore | null];
}>();

/** Long enough to see the ring land on the new tile before the sheet goes. */
const SETTLE_MS = 220;

const { t, locale } = useI18n();
const toast = useToast();

const candidates = ref<CoverCandidateOut[] | null>(null);
const partial = ref(false);
const error = ref("");
/** The key being saved: that tile is pressed, and every other choice waits. */
const pending = ref<string | null>(null);
const busy = computed(() => pending.value !== null);
/** How far each tile is down its sources (thumbnail, then the large picture); past the end it is hidden. */
const attempts = ref<Record<string, number>>({});
/** Tiles that had their one retry; the next failure moves on to the next source. */
const retried = new Set<string>();
/** Tiles waiting for their retry: the frame stays, empty. */
const waiting = ref<Record<string, boolean>>({});
/** Bumped per tile for the retry: a new `<img>`, so the browser asks the server again. */
const reloads = ref<Record<string, number>>({});
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();
let loadAbort: AbortController | null = null;
let settleTimer: ReturnType<typeof setTimeout> | null = null;

function sourcesOf(candidate: CoverCandidateOut): string[] {
  return coverSources({ title: props.title, cover_url: candidate.cover_url, cover_thumb_url: candidate.cover_thumb_url }, "sm");
}

const tiles = computed(() =>
  (candidates.value ?? []).flatMap((candidate) => {
    const src = sourcesOf(candidate)[attempts.value[candidate.key] ?? 0];
    if (!src) return [];
    const reload = reloads.value[candidate.key] ?? 0;
    return [{ candidate, src: waiting.value[candidate.key] ? null : src, reload }];
  }),
);

/** What Undo puts back after a change made here. */
const restore = computed<CoverRestore | null>(() => {
  if (!props.personalCover) return { key: null };
  const chosen = candidates.value?.find((candidate) => candidate.chosen);
  if (chosen) return { key: chosen.key };
  return props.coverPublicId ? { key: `media:${props.coverPublicId}` } : null;
});

/**
 * A tile's label in two lines: what tells editions apart first ("2019 ·
 * Deutsch"), then the publisher; the source's name when the edition says
 * nothing. A code that names no language (`und`, `mul`) is left out.
 */
function labelLines(candidate: CoverCandidateOut): { facts: string; publisher: string } {
  const edition = candidate.edition_label;
  const facts: string[] = [];
  if (edition?.year) facts.push(String(edition.year));
  if (edition?.language && isRealLanguage(edition.language)) facts.push(languageName(edition.language, locale.value));
  const publisher = edition?.publisher ? shortPublisher(edition.publisher) : "";
  if (!facts.length && !publisher) return { facts: t(`coverChoice.source.${candidate.source}`), publisher: "" };
  return { facts: facts.join(" · "), publisher };
}

/** The whole label on one line, for the tile's tooltip, with the publisher's full name. */
function label(candidate: CoverCandidateOut): string {
  const { facts } = labelLines(candidate);
  return [facts, candidate.edition_label?.publisher?.trim()].filter(Boolean).join(" · ");
}

function onImageError(candidate: CoverCandidateOut) {
  const key = candidate.key;
  if (retryTimers.has(key)) return;
  if (!retried.has(key)) {
    retried.add(key);
    waiting.value = { ...waiting.value, [key]: true };
    retryTimers.set(
      key,
      setTimeout(() => {
        retryTimers.delete(key);
        waiting.value = { ...waiting.value, [key]: false };
        reloads.value = { ...reloads.value, [key]: (reloads.value[key] ?? 0) + 1 };
      }, COVER_RETRY_MS),
    );
    return;
  }
  attempts.value = { ...attempts.value, [key]: (attempts.value[key] ?? 0) + 1 };
  reloads.value = { ...reloads.value, [key]: (reloads.value[key] ?? 0) + 1 };
}

/** A 1×1 placeholder is no cover either. */
function onImageLoad(event: Event, candidate: CoverCandidateOut) {
  const img = event.target as HTMLImageElement;
  if (img.naturalWidth === 1 && img.naturalHeight === 1) onImageError(candidate);
}

function reducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function settle(): Promise<void> {
  const wait = reducedMotion() ? 0 : SETTLE_MS;
  return new Promise((resolve) => {
    settleTimer = setTimeout(() => {
      settleTimer = null;
      resolve();
    }, wait);
  });
}

/** Save a choice (a candidate's key, or null for the standard cover), show it, and hand it to the page. */
async function apply(key: string | null) {
  if (busy.value) return;
  const before = restore.value;
  pending.value = key ?? "";
  try {
    const item = key ? await api.chooseCover(props.entryId, key) : await api.clearPersonalCover(props.entryId);
    if (candidates.value) {
      candidates.value = candidates.value.map((candidate) => ({ ...candidate, chosen: candidate.key === key }));
    }
    await settle();
    emit("changed", item, before);
    emit("close");
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("coverChoice.failed"));
  } finally {
    pending.value = null;
  }
}

function choose(candidate: CoverCandidateOut) {
  // The cover the reader sees already: nothing to change.
  if (candidate.chosen) {
    if (!busy.value) emit("close");
    return;
  }
  void apply(candidate.key);
}

/** A photo picked while a choice saves would be lost with the sheet: say so instead. */
function onPicked(file: File) {
  if (busy.value) toast.show(t("coverChoice.waitForSave"));
  else emit("picked", file, restore.value);
}

/** The sheet stays while a choice is saved, so its answer reaches the page. */
function requestClose() {
  if (!busy.value) emit("close");
}

onMounted(async () => {
  const controller = new AbortController();
  loadAbort = controller;
  try {
    const list = await api.coverCandidates(props.workId, { signal: controller.signal });
    candidates.value = list.items;
    partial.value = list.partial;
  } catch (err) {
    if (controller.signal.aborted) return;
    error.value = err instanceof ApiError ? err.message : t("coverChoice.loadFailed");
    candidates.value = [];
  }
});

onBeforeUnmount(() => {
  loadAbort?.abort();
  if (settleTimer !== null) clearTimeout(settleTimer);
  for (const timer of retryTimers.values()) clearTimeout(timer);
  retryTimers.clear();
});
</script>

<template>
  <BottomSheet class="cover-chooser" :title="t('coverChoice.title')" @close="requestClose">
    <p class="chooser-book">{{ title }}</p>
    <p class="fine subtle chooser-blurb">{{ t("coverChoice.blurb") }}</p>

    <ul v-if="personalCover" class="sheet-rows standard-rows">
      <li>
        <button class="sheet-row standard-cover" type="button" :disabled="busy" @click="apply(null)">
          <ArrowUturnLeftIcon class="sheet-icon" aria-hidden="true" />
          <span class="standard-text">
            <span class="sheet-label">{{ t("coverChoice.standard") }}</span>
            <span class="finer subtle">{{ t("coverChoice.standardHint") }}</span>
          </span>
        </button>
      </li>
    </ul>

    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <ul class="cover-grid" :aria-busy="candidates === null">
      <li class="grid-cell">
        <CoverPhotoButton
          pick-only
          look="tile"
          class="own-photo"
          :label="t('coverChoice.ownPhoto')"
          :work-id="workId"
          :edition-id="null"
          :has-cover="true"
          :disabled="busy"
          @picked="onPicked"
        >
          <template #icon><CameraIcon class="photo-tile-icon" aria-hidden="true" /></template>
        </CoverPhotoButton>
      </li>
      <template v-if="candidates === null">
        <li v-for="index in 5" :key="`skeleton-${index}`" class="grid-cell tile-skeleton" aria-hidden="true">
          <span class="skeleton tile-skeleton-cover" />
          <span class="skeleton skeleton-line short" />
        </li>
      </template>
      <li v-for="{ candidate, src, reload } in tiles" :key="candidate.key" class="grid-cell">
        <button
          class="cover-tile"
          :class="{ chosen: candidate.chosen, pressing: pending === candidate.key }"
          type="button"
          :title="label(candidate)"
          :aria-pressed="candidate.chosen"
          :disabled="busy && pending !== candidate.key"
          @click="choose(candidate)"
        >
          <span class="tile-frame">
            <img
              v-if="src"
              :key="reload"
              :src="src"
              alt=""
              loading="lazy"
              decoding="async"
              width="120"
              height="180"
              @error="onImageError(candidate)"
              @load="onImageLoad($event, candidate)"
            />
            <span v-if="candidate.chosen" class="tile-check">
              <CheckIcon aria-hidden="true" />
              <span class="visually-hidden">{{ t("coverChoice.chosen") }}</span>
            </span>
          </span>
          <span class="tile-label">
            <span class="label-facts">{{ labelLines(candidate).facts }}</span>
            <span v-if="labelLines(candidate).publisher" class="label-publisher">{{ labelLines(candidate).publisher }}</span>
          </span>
        </button>
      </li>
    </ul>

    <p v-if="candidates !== null && !error && tiles.length === 0" class="fine subtle chooser-note">
      {{ t("coverChoice.none") }}
    </p>
    <p v-if="partial" class="finer subtle chooser-note">{{ t("coverChoice.partial") }}</p>
  </BottomSheet>
</template>

<style scoped>
.chooser-book {
  margin: 0;
  font-family: var(--serif);
  color: var(--text-muted);
  overflow-wrap: anywhere;
}

.chooser-blurb {
  margin: var(--space-1) 0 var(--space-3);
}

.standard-rows {
  margin-bottom: var(--space-3);
  border-block: 1px solid var(--border);
}

.standard-text {
  display: grid;
  gap: 2px;
  min-width: 0;
}

/* Three covers across at 360 px, more on wider sheets. */
.cover-grid {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(92px, 1fr));
  gap: var(--space-3) var(--space-2);
}

.grid-cell {
  min-width: 0;
}

.cover-tile,
.own-photo {
  display: grid;
  gap: var(--space-1);
  width: 100%;
  min-height: var(--tap);
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: center;
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
}

.cover-tile {
  grid-template-columns: minmax(0, 1fr);
}

.tile-frame,
.own-photo {
  position: relative;
  display: block;
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-sm);
  background: var(--surface-2);
  transition:
    transform var(--dur-press) var(--ease-out),
    box-shadow var(--dur-base) var(--ease-out);
}

.tile-frame img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
}

.cover-tile:active:not(:disabled) .tile-frame,
.cover-tile.pressing .tile-frame,
.own-photo:active {
  transform: scale(var(--press-scale));
}

.cover-tile:focus-visible {
  outline: none;
}

.cover-tile:focus-visible .tile-frame {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}

.cover-tile:disabled {
  cursor: default;
  opacity: 0.55;
}

/* The chosen cover is ringed; the ring moves to a new choice before the sheet goes. */
.cover-tile.chosen .tile-frame {
  box-shadow:
    0 0 0 2px var(--bg),
    0 0 0 4px var(--accent);
}

.tile-check {
  position: absolute;
  top: 6px;
  right: 6px;
  display: grid;
  place-items: center;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  background: var(--accent);
  color: var(--accent-ink);
}

.tile-check svg {
  width: 16px;
  height: 16px;
  stroke-width: 2.5;
}

/* Year and language on the first line, the publisher on the second: the parts
   that tell editions apart are never cut off by a long publisher. */
.tile-label {
  display: grid;
  min-width: 0;
  font-size: var(--text-xs);
  line-height: 1.3;
  color: var(--text-muted);
}

.label-facts {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

/* Up to two lines that break between words; only a single word too long for
   the tile is cut, at its end. */
.label-publisher {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
  overflow-wrap: normal;
  word-break: normal;
  hyphens: manual;
}

.label-facts {
  font-weight: 600;
}

/* "Own photo": a dashed cover of its own, the camera over its words. */
.own-photo {
  display: grid;
  place-content: center;
  justify-items: center;
  border: 1.5px dashed var(--border-strong);
  background: transparent;
  color: var(--text-muted);
}

.own-photo :deep(input) {
  border-radius: inherit;
}

.photo-tile-icon {
  width: 26px;
  height: 26px;
}

.own-photo :deep(.photo-tile-label) {
  padding: 0 var(--space-1);
  font-size: var(--text-sm);
  font-weight: 600;
  line-height: 1.25;
}

@media (hover: hover) {
  .own-photo:hover,
  .cover-tile:hover:not(:disabled) .tile-label {
    color: var(--accent);
  }
}

.tile-skeleton {
  display: grid;
  gap: var(--space-1);
}

.tile-skeleton-cover {
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-sm);
}

.chooser-note {
  margin: var(--space-3) 0 0;
}

@media (prefers-reduced-motion: reduce) {
  .tile-frame,
  .own-photo {
    transition: none;
  }
}
</style>
