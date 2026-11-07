<script setup lang="ts">
import { ArrowDownIcon, ArrowUpIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { FAVORITE_LIMIT, bookPath } from "../constants";
import type { Favorite } from "../types";
import BookCover from "./BookCover.vue";

const { t } = useI18n();

const props = defineProps<{
  items: Favorite[];
  busy?: boolean;
}>();

const emit = defineEmits<{
  replace: [bookIds: number[]];
}>();

const ordered = computed(() =>
  [...props.items].sort((a, b) => a.position - b.position || a.book.id - b.book.id),
);

const slots = computed(() => {
  const byPosition = new Map(ordered.value.map((row) => [row.position, row]));
  return Array.from({ length: FAVORITE_LIMIT }, (_, index) => {
    const position = index + 1;
    return { position, row: byPosition.get(position) ?? null };
  });
});

const ids = computed(() => ordered.value.map((row) => row.book.id));

function move(index: number, delta: number) {
  const next = [...ids.value];
  const target = index + delta;
  if (target < 0 || target >= next.length) return;
  const [moved] = next.splice(index, 1);
  next.splice(target, 0, moved);
  emit("replace", next);
}

function clearAt(bookId: number) {
  emit(
    "replace",
    ids.value.filter((id) => id !== bookId),
  );
}

function clearAll() {
  emit("replace", []);
}
</script>

<template>
  <ol class="fav-list">
    <li v-for="slot in slots" :key="slot.position" class="fav-row" :class="{ empty: !slot.row }">
      <span class="fav-rank nums">
        <span aria-hidden="true">{{ slot.position }}</span>
        <span class="visually-hidden">{{ t("favorites.position", { n: slot.position }) }}</span>
      </span>
      <template v-if="slot.row">
        <RouterLink class="fav-cover" :to="bookPath(slot.row.book)" tabindex="-1" aria-hidden="true">
          <BookCover :book="slot.row.book" size="sm" />
        </RouterLink>
        <RouterLink class="fav-title" :to="bookPath(slot.row.book)">{{ slot.row.book.title }}</RouterLink>
        <div class="fav-actions">
          <button
            class="icon-btn"
            type="button"
            :disabled="busy || slot.position === 1"
            :aria-label="t('favorites.moveUp')"
            @click="move(slot.position - 1, -1)"
          >
            <ArrowUpIcon class="fav-icon" aria-hidden="true" />
          </button>
          <button
            class="icon-btn"
            type="button"
            :disabled="busy || slot.position === ids.length"
            :aria-label="t('favorites.moveDown')"
            @click="move(slot.position - 1, 1)"
          >
            <ArrowDownIcon class="fav-icon" aria-hidden="true" />
          </button>
          <button
            class="icon-btn"
            type="button"
            :disabled="busy"
            :aria-label="t('favorites.clearSlot')"
            @click="clearAt(slot.row.book.id)"
          >
            <XMarkIcon class="fav-icon" aria-hidden="true" />
          </button>
        </div>
      </template>
      <p v-else class="fine subtle fav-empty">{{ t("favorites.emptySlot") }}</p>
    </li>
  </ol>
  <button v-if="items.length" class="text-btn danger fav-clear-all" type="button" :disabled="busy" @click="clearAll">
    {{ t("favorites.clearAll") }}
  </button>
</template>

<style scoped>
/* Light rows with hairlines, like the other sheets: the place, the cover,
   the title, and the three moves under the title. */
.fav-list {
  list-style: none;
  margin: var(--space-2) 0 var(--space-3);
  padding: 0;
}

.fav-row {
  display: grid;
  grid-template-columns: 1.25rem auto minmax(0, 1fr);
  grid-template-areas:
    "rank cover title"
    "rank cover actions";
  column-gap: var(--space-3);
  align-items: center;
  padding: var(--space-3) 0;
  border-top: 1px solid var(--border);
}

.fav-row:first-child {
  border-top: 0;
}

.fav-row.empty {
  grid-template-areas: "rank empty empty";
  min-height: calc(var(--tap) + 2 * var(--space-3));
}

.fav-rank {
  grid-area: rank;
  align-self: center;
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 650;
  color: var(--text-muted);
  text-align: center;
}

.fav-cover {
  grid-area: cover;
  display: block;
  line-height: 0;
}

.fav-title {
  grid-area: title;
  align-self: end;
  font-family: var(--serif);
  font-weight: 650;
  color: inherit;
  text-decoration: none;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fav-actions {
  grid-area: actions;
  align-self: start;
  display: flex;
  gap: var(--space-1);
  margin-left: calc(-1 * var(--space-3));
}

.fav-icon {
  width: 1.25rem;
  height: 1.25rem;
}

.icon-btn:disabled {
  opacity: 0.35;
}

.fav-empty {
  grid-area: empty;
  margin: 0;
}

/* The padding only widens the target: the text lines up with the rows. */
.fav-clear-all {
  justify-self: start;
  min-height: var(--tap);
  margin-inline-start: calc(-1 * var(--space-2));
}
</style>
