<script setup lang="ts">
import { ChevronDownIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "vue-i18n";
import { computed } from "vue";
import { statusLabel } from "../constants";
import { readingMonth, readingMonthLabel } from "../shelf";
import type { ShelfItem } from "../types";
import BookTile from "./BookTile.vue";

const props = withDefaults(defineProps<{
  item: ShelfItem;
  readonly?: boolean;
  disabled?: boolean;
  /** Works that are a current pick in one of the viewer's clubs. */
  clubPickIds?: readonly number[];
  timezone?: string;
  /**
   * In the finished grid: the month marker over this card ("Okt. 2026" on the
   * first book of a month, "" on the others, which keep the line so the rows
   * stay level). Elsewhere absent.
   */
  marker?: string;
}>(), { timezone: "UTC", marker: undefined });
const emit = defineEmits<{
  action: [action: "status" | "date" | "previous" | "next"];
}>();
const { t, locale } = useI18n();
const month = computed(() => readingMonth(props.item, props.timezone));
const monthLabel = computed(() => readingMonthLabel(month.value, locale.value));
/** The chip's words as shown ("Aktuell 35 %"), so its spoken name starts with them. */
const shownStatus = computed(() => {
  const name = statusLabel(props.item.status);
  return props.item.status === "currently_reading" && props.item.progress != null
    ? `${name} ${t("progress.percent", { n: props.item.progress })}`
    : name;
});
</script>

<template>
  <article class="shelf-card" :class="{ 'readonly-card': readonly }">
    <!-- Seen, not read out: the card's own month (below) says it for each book. -->
    <span v-if="marker !== undefined" class="month-marker" :class="{ starts: marker }" aria-hidden="true">{{ marker }}</span>
    <BookTile :book="item.book" :status="item.status" :rating="item.rating" hide-status
      :club-pick="clubPickIds?.includes(item.book.id)" show-authors />
    <span v-if="readonly" class="shelf-status">
      <span class="status-label">{{ statusLabel(item.status) }}<span v-if="item.status === 'currently_reading' && item.progress != null" class="nums"> {{ t('progress.percent', { n: item.progress }) }}</span></span>
    </span>
    <button v-else class="shelf-status no-drag" type="button"
      :disabled="disabled" aria-haspopup="dialog"
      :aria-label="t('shelf.statusFor', { title: item.book.title, status: shownStatus })"
      @click="emit('action', 'status')">
      <span class="status-label">{{ statusLabel(item.status) }}<span v-if="item.status === 'currently_reading' && item.progress != null" class="nums"> {{ t('progress.percent', { n: item.progress }) }}</span></span>
      <ChevronDownIcon class="status-chevron" aria-hidden="true" />
    </button>
    <p v-if="item.status === 'finished'" class="visually-hidden">
      <time v-if="month" :datetime="month" :aria-label="t('shelf.finishedMonth', { month: monthLabel })">{{ monthLabel }}</time>
      <span v-else>{{ t('shelf.noReadingDate') }}</span>
    </p>
  </article>
</template>

<style scoped>
.shelf-card { position: relative; min-width: 0; display: flex; flex-direction: column; cursor: grab; container-type: inline-size; }
.readonly-card { cursor: auto; }
/* A ruled line over each finished cover; the first book of a month carries the
   month on a darker stretch of it, so the months read as marks in one flow. */
.month-marker {
  display: block;
  min-height: calc(var(--text-xs) * 1.3 + var(--space-1));
  margin-bottom: var(--space-2);
  padding-top: var(--space-1);
  border-top: 1px solid var(--border);
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: 600;
  line-height: 1.3;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.month-marker.starts { border-top-color: var(--text-muted); }
.shelf-card :deep(.book-tile) { user-select: none; -webkit-user-select: none; -webkit-touch-callout: none; flex: 1; gap: 4px; padding-bottom: var(--space-2); }
.shelf-card :deep(img) { -webkit-user-drag: none; }
.shelf-card :deep(.book-tile-cover) { margin-bottom: var(--space-1); }
.shelf-card :deep(.book-tile-title) { font-size: var(--text-sm); line-height: 1.3; min-height: 2.6em; }
.shelf-card :deep(.book-tile-sub) { font-size: var(--text-xs); color: var(--text-muted); }
.shelf-card :deep(.stars) { color: var(--accent); }
.shelf-status {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-1);
  width: 100%;
  min-height: var(--tap);
  padding: var(--space-2) 0;
  border: 0;
  border-top: 1px solid var(--border-strong);
  border-radius: 0;
  background: transparent;
  color: var(--text);
  font-family: var(--serif);
  font-size: var(--text-sm);
  font-weight: 400;
  text-align: left;
  line-height: var(--leading-snug);
}
.status-label { min-width: 0; hyphens: auto; }
.status-label .nums { white-space: nowrap; margin-left: var(--space-1); font-size: var(--text-xs); }
.status-chevron { width: 16px; height: 16px; flex-shrink: 0; color: var(--text-muted); }
/* In the phone's three-up grid a card is about 96 px wide, and "Abgebrochen"
   (89 px at 14 px) with its chevron needs 109: there the chip's words go a
   step smaller and the chevron closer, so the word stays whole and clear of
   it (the card is the container). A word still too long (a large text
   setting) hyphenates. */
@container (max-width: 108px) {
  .shelf-status { gap: 2px; }
  .status-label { font-size: var(--text-xs); }
  .status-chevron { width: 14px; height: 14px; }
}
button.shelf-status { cursor: pointer; transition: color var(--dur-fast) var(--ease); }
@media (hover: hover) {
  button.shelf-status:hover:not(:disabled) { color: var(--accent); }
}
@media (hover: hover) {
  button.shelf-status:hover:not(:disabled) .status-chevron { color: inherit; }
}
button.shelf-status:disabled { cursor: wait; opacity: 0.6; }
</style>
