<script setup lang="ts">
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
  <ul class="fav-list">
    <li v-for="slot in slots" :key="slot.position" class="fav-row">
      <span class="fav-rank nums">{{ t("favorites.position", { n: slot.position }) }}</span>
      <template v-if="slot.row">
        <RouterLink class="fav-book" :to="bookPath(slot.row.book.ol_work_key)">
          <BookCover
            :title="slot.row.book.title"
            :cover-id="slot.row.book.cover_id"
            :work-key="slot.row.book.ol_work_key"
            :image-url="slot.row.book.cover_url"
            size="sm"
          />
          <span class="fav-title">{{ slot.row.book.title }}</span>
        </RouterLink>
        <div class="fav-actions">
          <button
            class="btn btn-ghost btn-sm"
            type="button"
            :disabled="busy || slot.position === 1"
            :aria-label="t('favorites.moveUp')"
            @click="move(slot.position - 1, -1)"
          >
            ↑
          </button>
          <button
            class="btn btn-ghost btn-sm"
            type="button"
            :disabled="busy || slot.position === ids.length"
            :aria-label="t('favorites.moveDown')"
            @click="move(slot.position - 1, 1)"
          >
            ↓
          </button>
          <button
            class="btn btn-ghost btn-sm"
            type="button"
            :disabled="busy"
            :aria-label="t('favorites.clearSlot')"
            @click="clearAt(slot.row.book.id)"
          >
            {{ t("common.clear") }}
          </button>
        </div>
      </template>
      <p v-else class="fine subtle fav-empty">{{ t("favorites.emptySlot") }}</p>
    </li>
  </ul>
  <button
    v-if="items.length"
    class="btn btn-ghost"
    type="button"
    :disabled="busy"
    @click="clearAll"
  >
    {{ t("favorites.clearAll") }}
  </button>
</template>

<style scoped>
.fav-list {
  list-style: none;
  margin: var(--space-3) 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.fav-row {
  display: grid;
  grid-template-columns: 6.5rem minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.fav-rank {
  font-size: var(--text-xs);
  font-weight: 700;
  color: var(--text-muted);
}

.fav-book {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
  color: inherit;
  text-decoration: none;
}

.fav-title {
  font-family: var(--serif);
  font-weight: 650;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fav-empty {
  grid-column: 2 / -1;
  margin: 0;
}

.fav-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  justify-self: end;
}

@media (max-width: 639px) {
  .fav-row {
    grid-template-columns: auto minmax(0, 1fr);
  }

  .fav-actions {
    grid-column: 1 / -1;
    justify-self: start;
  }
}
</style>
