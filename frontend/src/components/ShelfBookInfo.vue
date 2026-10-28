<script setup lang="ts">
import { useI18n } from "vue-i18n";
import BookCover from "./BookCover.vue";
import { starLabel, statusLabel } from "../constants";
import type { ShelfItem } from "../types";

defineProps<{ item: ShelfItem }>();
const { t } = useI18n();
</script>

<template>
  <div class="shelf-book-info">
    <div class="spine-sheet-book">
      <BookCover :book="item.book" size="md" />
      <div>
        <h3>{{ item.book.title }}</h3>
        <p class="muted">{{ item.book.authors }}</p>
        <p v-if="item.status === 'finished' && item.rating" class="spine-sheet-stars" :aria-label="t('book.starsOf', { n: item.rating })">{{ starLabel(item.rating) }}</p>
        <span class="finer subtle">{{ statusLabel(item.status) }}</span>
        <span v-if="item.status === 'currently_reading' && item.progress != null" class="sheet-progress">
          <span class="progress-track" aria-hidden="true"><span class="progress-fill" :style="{ width: `${item.progress}%` }" /></span>
          <span class="finer subtle nums">{{ item.progress }}%</span>
        </span>
      </div>
      <slot />
    </div>
    <p v-if="item.take" class="sheet-take">{{ item.take }}</p>
  </div>
</template>

<style scoped>
.spine-sheet-book { display: flex; gap: 14px; align-items: center; }
.spine-sheet-book > div { flex: 1; min-width: 0; }
.spine-sheet-book h3 { overflow-wrap: anywhere; }
.spine-sheet-stars { color: var(--accent); font-size: var(--text-sm); }
.sheet-progress { display: flex; align-items: center; gap: var(--space-2); margin-top: var(--space-1); }
.progress-track { flex: 1; max-width: 140px; height: 4px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
.progress-fill { display: block; height: 100%; border-radius: inherit; background: var(--accent); }
.sheet-take { margin-top: var(--space-4); padding-left: var(--space-3); border-left: 2px solid var(--accent-line); color: var(--text-muted); font-family: var(--serif); font-style: italic; overflow-wrap: anywhere; white-space: pre-wrap; }
</style>
