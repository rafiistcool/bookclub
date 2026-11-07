<script setup lang="ts">
import { ref, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import type { ShelfItem } from "../types";
import BookCard from "./BookCard.vue";

/**
 * One draggable grid of shelf cards. With `markers` it is the finished grid:
 * every card gets a marker line (the month's words on the first book of a
 * month), and its books never mix with the unfinished ones.
 */
const props = defineProps<{
  items: ShelfItem[]; markers?: Record<number, string>; readonly?: boolean; disabled?: boolean;
  clubPickIds?: readonly number[]; timezone: string;
}>();
const emit = defineEmits<{
  action: [item: ShelfItem, action: "status" | "date" | "previous" | "next"];
  moved: [item: ShelfItem, rows: ShelfItem[]];
  /** During a drag: the order with the book in the slot it hovers; null when it ends. */
  preview: [state: { rows: ShelfItem[]; item: ShelfItem } | null];
}>();
const rows = ref<ShelfItem[]>([]);
watch(() => props.items, items => { rows.value = [...items]; }, { immediate: true, deep: true });
function moved(event: { newDraggableIndex?: number; newIndex?: number }) {
  if (props.readonly || props.disabled) return;
  const item = rows.value[event.newDraggableIndex ?? event.newIndex ?? 0];
  if (item) emit("moved", item, rows.value);
}

type DragEvent = { oldDraggableIndex?: number; oldIndex?: number; newDraggableIndex?: number; newIndex?: number };
let lifted: ShelfItem | null = null;
function started(event: DragEvent) {
  lifted = rows.value[event.oldDraggableIndex ?? event.oldIndex ?? -1] ?? null;
}
/** Sortable moves the card in the page as it goes; the list itself changes only on the drop. */
function changed(event: DragEvent) {
  const item = lifted;
  const index = event.newDraggableIndex ?? event.newIndex;
  if (!item || index === undefined) return;
  const order = rows.value.filter(row => row.id !== item.id);
  order.splice(index, 0, item);
  emit("preview", { rows: order, item });
}
function ended() {
  if (!lifted) return;
  lifted = null;
  emit("preview", null);
}
</script>

<template>
  <VueDraggable v-model="rows" class="book-grid shelf-grid"
    :group="markers ? 'shelf-finished' : 'shelf-active'"
    :animation="180" :disabled="readonly || disabled" :delay="250" :delay-on-touch-only="true"
    :touch-start-threshold="5" :fallback-tolerance="5"
    :prevent-on-filter="false" filter=".no-drag" draggable=".shelf-card"
    ghost-class="card-ghost" chosen-class="card-chosen" fallback-class="card-lifted"
    @start="started" @change="changed" @end="ended" @update="moved">
    <BookCard v-for="item in rows" :key="item.id" :data-id="item.id"
      :timezone="timezone" :item="item" :readonly="readonly" :disabled="disabled" :club-pick-ids="clubPickIds"
      :marker="markers ? (markers[item.id] ?? '') : undefined"
      @action="emit('action', item, $event)" />
  </VueDraggable>
</template>

<style scoped>
.shelf-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); align-items: stretch; gap: var(--space-6) var(--space-5); }
.card-ghost { opacity: 0.3; }
/* The card under the finger is a copy taken at the lift: its month marker
   would name the month it left, so only the ruled line goes along. */
.card-lifted :deep(.month-marker) { color: transparent; }
.card-chosen { cursor: grabbing; }
@media (max-width: 359px) {
  .shelf-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
@media (min-width: 720px) {
  .shelf-grid { grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); }
}
</style>
