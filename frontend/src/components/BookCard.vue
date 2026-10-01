<script setup lang="ts">
import { ChevronDownIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "vue-i18n";
import { computed } from "vue";
import { statusLabel, statusShort } from "../constants";
import { readingMonth, readingMonthLabel } from "../shelf";
import type { ShelfItem } from "../types";
import BookTile from "./BookTile.vue";

const props = withDefaults(defineProps<{
  item: ShelfItem;
  readonly?: boolean;
  disabled?: boolean;
  clubPickKey?: string | null;
  timezone?: string;
}>(), { timezone: "UTC" });
const emit = defineEmits<{
  action: [action: "status" | "date" | "previous" | "next"];
}>();
const { t, locale } = useI18n();
const month = computed(() => readingMonth(props.item, props.timezone));
const monthLabel = computed(() => readingMonthLabel(month.value, locale.value));
</script>

<template>
  <article class="shelf-card" :class="{ 'readonly-card': readonly }">
    <BookTile :ol-work-key="item.book.ol_work_key" :title="item.book.title"
      :authors="item.book.authors" :cover-id="item.book.cover_id"
      :image-url="item.book.cover_url" :status="item.status" :rating="item.rating" hide-status
      :club-pick="item.book.ol_work_key === clubPickKey" show-authors />
    <span v-if="readonly" class="shelf-status">
      <span class="status-label">{{ statusShort(item.status) }}<span v-if="item.status === 'currently_reading' && item.progress != null" class="nums"> {{ item.progress }}%</span></span>
    </span>
    <button v-else class="shelf-status no-drag" type="button"
      :disabled="disabled" aria-haspopup="dialog"
      :aria-label="t('shelf.statusFor', { title: item.book.title, status: statusLabel(item.status) })"
      @click="emit('action', 'status')">
      <span class="status-label">{{ statusShort(item.status) }}<span v-if="item.status === 'currently_reading' && item.progress != null" class="nums"> {{ item.progress }}%</span></span>
      <ChevronDownIcon class="status-chevron" aria-hidden="true" />
    </button>
    <p v-if="item.status === 'finished'" class="visually-hidden">
      <time v-if="month" :datetime="month" :aria-label="t('shelf.finishedMonth', { month: monthLabel })">{{ monthLabel }}</time>
      <span v-else>{{ t('shelf.noReadingDate') }}</span>
    </p>
  </article>
</template>

<style scoped>
.shelf-card { position: relative; min-width: 0; display: flex; flex-direction: column; cursor: grab; }
.readonly-card { cursor: auto; }
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
.status-label { min-width: 0; }
.status-label .nums { white-space: nowrap; margin-left: var(--space-1); font-size: var(--text-xs); }
.status-chevron { width: 16px; height: 16px; flex-shrink: 0; color: var(--text-muted); }
button.shelf-status { cursor: pointer; transition: color var(--dur-fast) var(--ease); }
button.shelf-status:hover:not(:disabled) { color: var(--accent); }
button.shelf-status:hover:not(:disabled) .status-chevron { color: inherit; }
button.shelf-status:disabled { cursor: wait; opacity: 0.6; }
</style>
