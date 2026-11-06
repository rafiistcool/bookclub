<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { tp } from "../i18n";
import { useFinish } from "../stores/finish";
import type { ShelfItem } from "../types";
import BottomSheet from "./BottomSheet.vue";

/**
 * A reader's progress as a row that opens a sheet, never a slider on the
 * page: a stray thumb on a range input used to save (and post to the club)
 * at once. The sheet counts in pages when the book has a page count, and
 * saves only on "Save". The server keeps a percent; pages are derived.
 *
 * With `shelfId`, a draft at the last page offers "Mark as read": the editor
 * itself moves the entry to finished (no progress 100 first, so no progress
 * event and no progress Undo), opens the finished moment and emits
 * `finished` with the stored entry. "Just save progress" keeps the old save.
 */
const props = defineProps<{
  value: number | null;
  pages?: number | null;
  label: string;
  bookTitle: string;
  busy?: boolean;
  /** The reader's shelf entry: with it, the last page offers to mark the book as read. */
  shelfId?: number | null;
  /** The club whose current pick this is, for the moment's "2 of 4" line. */
  clubSlug?: string | null;
}>();

const emit = defineEmits<{ save: [percent: number]; finished: [item: ShelfItem] }>();
const { t } = useI18n();

const open = ref(false);
const percent = ref(0);
const trigger = ref<HTMLButtonElement | null>(null);
const page = ref(0);

const pageCount = computed(() => (props.pages && props.pages > 0 ? props.pages : null));
const stored = computed(() => Math.min(100, Math.max(0, Math.round(props.value ?? 0))));

function pageOf(value: number): number {
  return pageCount.value ? Math.round((value / 100) * pageCount.value) : 0;
}

// The page derived from the stored percent stands for that percent: with
// fewer than 100 pages the round trip would otherwise move it (35 % of 50
// pages is page 18, which is 36 %) and offer to save a value nobody chose.
const draft = computed(() => {
  if (!pageCount.value) return percent.value;
  if (page.value === pageOf(stored.value)) return stored.value;
  return Math.min(100, Math.round((page.value / pageCount.value) * 100));
});
const changed = computed(() => draft.value !== stored.value);
/** At the end of the book: "Mark as read" takes the primary place. */
const offersFinish = computed(() => props.shelfId != null && draft.value >= 100);
const finishing = ref(false);
const finishError = ref("");

function start() {
  if (props.busy || finishing.value) return;
  finishError.value = "";
  percent.value = stored.value;
  page.value = pageOf(stored.value);
  open.value = true;
}

function step(by: number) {
  if (pageCount.value) page.value = Math.min(pageCount.value, Math.max(0, page.value + by));
  else percent.value = Math.min(100, Math.max(0, percent.value + by));
  tick();
}

function onRange(event: Event) {
  const value = Number((event.target as HTMLInputElement).value);
  if (pageCount.value) page.value = value;
  else percent.value = value;
}

function onPageInput(event: Event) {
  const value = Number.parseInt((event.target as HTMLInputElement).value, 10);
  if (Number.isFinite(value) && pageCount.value) page.value = Math.min(pageCount.value, Math.max(0, value));
}

// A short tick under the finger on phones that have it (Android); a no-op
// elsewhere.
function tick() {
  if (typeof navigator !== "undefined" && typeof navigator.vibrate === "function") navigator.vibrate(6);
}

function save() {
  if (finishing.value) return;
  open.value = false;
  if (changed.value) emit("save", draft.value);
}

function close() {
  if (!finishing.value) open.value = false;
}

/** One PATCH to finished; the sheet closes first, so the moment never stacks on it. */
async function markFinished() {
  const id = props.shelfId;
  if (id == null || finishing.value) return;
  finishing.value = true;
  finishError.value = "";
  try {
    const item = await api.patchShelf(id, { status: "finished", position: 0 });
    // Noted before `finished`: the parent usually removes this editor with the book.
    const opener = trigger.value;
    open.value = false;
    emit("finished", item);
    // Looked up here, not at setup: a parent without a finish path needs no store.
    void useFinish().celebrate({ item, clubSlug: props.clubSlug ?? null, opener });
  } catch (err) {
    finishError.value = err instanceof ApiError ? err.message : t("book.saveFailed");
  } finally {
    finishing.value = false;
  }
}

const valueText = computed(() =>
  pageCount.value
    ? `${t("progress.page", { page: page.value, pages: pageCount.value })}, ${t("progress.percent", { n: draft.value })}`
    : t("progress.percent", { n: percent.value }),
);

const steps = computed(() => (pageCount.value ? [-10, -1, 1, 10] : [-5, -1, 1, 5]));

function stepLabel(by: number): string {
  const n = Math.abs(by);
  if (pageCount.value) return tp(by < 0 ? "progress.backPages" : "progress.forwardPages", n);
  return t(by < 0 ? "progress.backPercent" : "progress.forwardPercent", { n });
}
</script>

<template>
  <button
    ref="trigger"
    class="progress-trigger"
    type="button"
    aria-haspopup="dialog"
    :aria-disabled="busy || finishing || undefined"
    :aria-label="t('progress.change', { label, n: stored })"
    @click="start"
  >
    <span class="progress-line">
      <span class="finer subtle">{{ label }}</span>
      <span class="progress-figure">
        <span v-if="pageCount" class="finer subtle nums">{{ t("progress.page", { page: pageOf(stored), pages: pageCount }) }}</span>
        <strong class="nums fine">{{ t("progress.percent", { n: stored }) }}</strong>
      </span>
    </span>
    <span class="progress-track" aria-hidden="true">
      <span class="progress-fill" :style="{ transform: `scaleX(${stored / 100})` }" />
    </span>
  </button>

  <BottomSheet v-if="open" :title="label" @close="close">
    <p class="progress-book">{{ bookTitle }}</p>
    <span class="visually-hidden" aria-live="polite" aria-atomic="true">{{ valueText }}</span>
    <div class="progress-readout">
      <template v-if="pageCount">
        <label class="page-field">
          <span class="visually-hidden">{{ t("progress.pageLabel") }}</span>
          <input
            class="page-input nums"
            type="number"
            inputmode="numeric"
            min="0"
            :max="pageCount"
            :value="page"
            @input="onPageInput"
          />
        </label>
        <span class="readout-of nums">{{ t("progress.ofPages", { pages: pageCount }) }}</span>
        <span class="readout-percent nums">{{ t("progress.percent", { n: draft }) }}</span>
      </template>
      <template v-else>
        <strong class="readout-big nums">{{ percent }}</strong><span class="readout-unit">%</span>
      </template>
    </div>
    <input
      class="progress-range"
      type="range"
      min="0"
      :max="pageCount ?? 100"
      step="1"
      :value="pageCount ? page : percent"
      :aria-label="label"
      :aria-valuetext="valueText"
      :style="{ '--fill': `${draft}%` }"
      @input="onRange"
    />
    <div class="progress-steps">
      <button
        v-for="by in steps"
        :key="by"
        class="btn btn-ghost nums"
        type="button"
        :aria-label="stepLabel(by)"
        @click="step(by)"
      >
        {{ by > 0 ? `+${by}` : `−${-by}` }}
      </button>
    </div>
    <template v-if="offersFinish">
      <p v-if="finishError" class="error" role="alert">{{ finishError }}</p>
      <div class="progress-actions">
        <button class="btn btn-primary btn-block" type="button" :disabled="finishing" @click="markFinished">
          {{ t("progress.markFinished") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" :disabled="!changed || finishing" @click="save">
          {{ t("progress.saveOnly") }}
        </button>
      </div>
    </template>
    <button v-else class="btn btn-primary btn-block" type="button" :disabled="!changed" @click="save">
      {{ t("common.save") }}
    </button>
  </BottomSheet>
</template>

<style scoped>
.progress-trigger {
  display: grid;
  gap: var(--space-2);
  width: 100%;
  min-height: var(--tap);
  margin-top: var(--space-1);
  padding: var(--space-2) 0;
  border: 0;
  background: transparent;
  color: inherit;
  text-align: left;
  transition: transform var(--dur-fast) var(--ease-out);
}

.progress-trigger:active:not([aria-disabled="true"]) {
  transform: scale(0.98);
  transition-duration: var(--dur-press);
}

/* Not disabled: a disabled button drops the focus the sheet hands back. */
.progress-trigger[aria-disabled="true"] {
  cursor: wait;
  opacity: 0.6;
}

@media (prefers-reduced-motion: reduce) {
  .progress-trigger:active:not([aria-disabled="true"]) {
    transform: none;
  }
}

.progress-line {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-2);
}

.progress-figure {
  display: inline-flex;
  align-items: baseline;
  gap: var(--space-2);
}

/* A bookmark ribbon of a bar: thin, rounded, filling from the spine side. */
.progress-track {
  position: relative;
  display: block;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  overflow: hidden;
}

.progress-fill {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: var(--accent);
  transform-origin: left center;
  transition: transform 600ms var(--ease-out);
}

.progress-book {
  margin-bottom: var(--space-4);
  color: var(--text-muted);
  font-family: var(--serif);
}

.progress-readout {
  display: flex;
  align-items: baseline;
  justify-content: center;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}

.readout-big {
  font-family: var(--serif);
  font-size: var(--text-4xl);
  line-height: 1;
}

.readout-unit,
.readout-of {
  font-size: var(--text-xl);
  color: var(--text-muted);
}

.readout-percent {
  margin-left: var(--space-3);
  font-weight: 600;
  color: var(--accent-hover);
}

.page-input {
  width: 5ch;
  padding: 0;
  border: 0;
  border-bottom: 2px solid var(--accent-line);
  background: transparent;
  font-family: var(--serif);
  font-size: var(--text-4xl);
  line-height: 1;
  text-align: center;
  appearance: textfield;
  -moz-appearance: textfield;
}

.page-input::-webkit-inner-spin-button,
.page-input::-webkit-outer-spin-button {
  -webkit-appearance: none;
  margin: 0;
}

.page-input:focus-visible {
  outline: none;
  border-bottom-color: var(--accent);
}

/* A range the thumb can actually hit: a 28px knob on a filled track. */
.progress-range {
  --fill: 0%;
  width: 100%;
  height: var(--tap);
  margin: 0 0 var(--space-3);
  background: transparent;
  appearance: none;
  -webkit-appearance: none;
}

.progress-range::-webkit-slider-runnable-track {
  height: 8px;
  border-radius: var(--radius-pill);
  background: linear-gradient(to right, var(--accent) var(--fill), var(--surface-3) var(--fill));
}

.progress-range::-moz-range-track {
  height: 8px;
  border-radius: var(--radius-pill);
  background: var(--surface-3);
}

.progress-range::-moz-range-progress {
  height: 8px;
  border-radius: var(--radius-pill);
  background: var(--accent);
}

.progress-range::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 28px;
  height: 28px;
  margin-top: -10px;
  border: 3px solid var(--bg);
  border-radius: 50%;
  background: var(--accent);
  box-shadow: var(--elev-panel);
}

.progress-range::-moz-range-thumb {
  width: 22px;
  height: 22px;
  border: 3px solid var(--bg);
  border-radius: 50%;
  background: var(--accent);
  box-shadow: var(--elev-panel);
}

.progress-actions {
  display: grid;
  gap: var(--space-2);
}

.progress-steps {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}
</style>
